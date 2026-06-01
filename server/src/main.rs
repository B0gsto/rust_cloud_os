use anyhow::Context;
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use aws_sdk_dynamodb::{Client as Db, types::AttributeValue as Av};
use aws_sdk_s3::{Client as S3, primitives::ByteStream};
use axum::{
    Json, Router,
    body::Body,
    extract::{
        DefaultBodyLimit, Path, Query, Request, State,
        ws::{Message, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{Duration, Utc};
use futures_util::{SinkExt, StreamExt};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, env, net::SocketAddr};
use tokio_util::io::ReaderStream;
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

#[derive(Clone)]
struct App {
    db: Db,
    s3: S3,
    table: String,
    bucket: String,
    jwt: String,
    free: i64,
}
#[derive(Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
}
#[derive(Deserialize)]
struct Auth {
    email: String,
    password: String,
}
#[derive(Serialize)]
struct Me {
    email: String,
    used: i64,
    free: i64,
    paid: bool,
}
#[derive(Serialize)]
struct File {
    name: String,
    bytes: i64,
}

const JIT_OS_SNAPSHOT: &str = "snapshots/jit-os-overlay.bin";
const SYSTEM_JIT_OS_SNAPSHOT: &str = "system/snapshots/jit-os-overlay.bin";

type Api<T> = Result<T, (StatusCode, String)>;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let app = App {
        db: Db::new(&cfg),
        s3: S3::new(&cfg),
        table: env::var("USERS_TABLE").context("USERS_TABLE missing")?,
        bucket: env::var("S3_BUCKET").context("S3_BUCKET missing")?,
        jwt: env::var("JWT_SECRET").context("JWT_SECRET missing")?,
        free: env::var("FREE_GB")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5)
            * 1_073_741_824,
    };
    let static_files = ServeDir::new("server/web-dist")
        .not_found_service(ServeFile::new("server/web-dist/index.html"));
    let routes = Router::new()
        .route("/api/signup", post(signup))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/files", get(files))
        .route(
            "/api/upload",
            post(upload).layer(DefaultBodyLimit::max(2_500_000_000)),
        )
        .route("/api/file/{name}", get(download).delete(delete_file))
        .route("/api/system/{name}", get(download_system))
        .route(
            "/api/vm/snapshot",
            get(vm_snapshot)
                .post(save_vm_snapshot)
                .layer(DefaultBodyLimit::max(2_500_000_000)),
        )
        .route("/api/upgrade", post(upgrade))
        .route("/api/vm/net", get(vm_net))
        .fallback_service(static_files)
        .layer(middleware::from_fn(cross_origin_isolation_headers))
        .layer(TraceLayer::new_for_http())
        .with_state(app);
    axum::serve(
        tokio::net::TcpListener::bind("0.0.0.0:3000").await?,
        routes.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

async fn cross_origin_isolation_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let headers = res.headers_mut();
    headers.insert(
        "Cross-Origin-Opener-Policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        "Cross-Origin-Embedder-Policy",
        HeaderValue::from_static("require-corp"),
    );
    headers.insert("Origin-Agent-Cluster", HeaderValue::from_static("?1"));
    res
}

async fn signup(State(a): State<App>, Json(x): Json<Auth>) -> Api<impl IntoResponse> {
    let email = sanitize_email(&x.email)?;
    let salt = SaltString::generate(&mut OsRng);
    let pass = Argon2::default()
        .hash_password(x.password.as_bytes(), &salt)
        .map_err(e)?
        .to_string();
    a.db.put_item()
        .table_name(&a.table)
        .item("email", Av::S(email.clone()))
        .item("pass", Av::S(pass))
        .item("used", Av::N("0".into()))
        .item("paid", Av::Bool(false))
        .condition_expression("attribute_not_exists(email)")
        .send()
        .await
        .map_err(e)?;
    ensure_user_snapshot(&a, &email).await?;
    Ok(StatusCode::CREATED)
}
async fn login(State(a): State<App>, Json(x): Json<Auth>) -> Api<impl IntoResponse> {
    let email = sanitize_email(&x.email)?;
    let u = user(&a, &email).await?;
    let pass = u
        .get("pass")
        .and_then(|v| v.as_s().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "bad login".into()))?;
    Argon2::default()
        .verify_password(x.password.as_bytes(), &PasswordHash::new(pass).map_err(e)?)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "bad login".into()))?;
    let c = Claims {
        sub: email,
        exp: (Utc::now() + Duration::days(7)).timestamp() as usize,
    };
    let token = encode(
        &Header::default(),
        &c,
        &EncodingKey::from_secret(a.jwt.as_bytes()),
    )
    .map_err(e)?;
    Ok((
        [(
            header::SET_COOKIE,
            format!("sid={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age=604800"),
        )],
        StatusCode::OK,
    ))
}
async fn logout() -> impl IntoResponse {
    [(
        header::SET_COOKIE,
        "sid=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0",
    )]
}
async fn me(State(a): State<App>, h: HeaderMap) -> Json<Option<Me>> {
    let email = match auth(&a, &h) {
        Ok(email) => email,
        Err(_) => return Json(None),
    };
    let u = match user(&a, &email).await {
        Ok(u) => u,
        Err(_) => return Json(None),
    };
    Json(Some(Me {
        email,
        used: num(&u, "used"),
        free: a.free,
        paid: paid(&u),
    }))
}
async fn files(State(a): State<App>, h: HeaderMap) -> Api<Json<Vec<File>>> {
    let email = auth(&a, &h)?;
    let r =
        a.s3.list_objects_v2()
            .bucket(&a.bucket)
            .prefix(format!("{email}/"))
            .send()
            .await
            .map_err(e)?;
    Ok(Json(
        r.contents()
            .iter()
            .filter_map(|o| {
                let name = o
                    .key()
                    .unwrap_or_default()
                    .trim_start_matches(&format!("{email}/"))
                    .to_string();
                if name.starts_with("snapshots/") {
                    None
                } else {
                    Some(File {
                        name,
                        bytes: o.size().unwrap_or(0),
                    })
                }
            })
            .collect(),
    ))
}
async fn upload(
    State(a): State<App>,
    h: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    b: Body,
) -> Api<impl IntoResponse> {
    let email = auth(&a, &h)?;
    let name = sanitize_file_name(
        q.get("name")
            .ok_or((StatusCode::BAD_REQUEST, "missing name".into()))?,
    )?;
    let new_size = content_length(&h)?;

    let key = format!("{email}/{name}");
    let old_size = match a.s3.head_object().bucket(&a.bucket).key(&key).send().await {
        Ok(resp) => resp.content_length().unwrap_or(0),
        Err(_) => 0,
    };
    let size_diff = new_size - old_size;

    reserve_storage(&a, &email, size_diff).await?;

    let put_result =
        a.s3.put_object()
            .bucket(&a.bucket)
            .key(&key)
            .content_length(new_size)
            .body(byte_stream_from_axum_body(b))
            .send()
            .await;

    if let Err(err) = put_result {
        let _ = reserve_storage(&a, &email, -size_diff).await;
        return Err(e(err));
    }

    Ok(StatusCode::CREATED)
}
async fn download(
    State(a): State<App>,
    h: HeaderMap,
    Path(name): Path<String>,
) -> Api<impl IntoResponse> {
    let email = auth(&a, &h)?;
    let name = sanitize_file_name(&name)?;
    let o =
        a.s3.get_object()
            .bucket(&a.bucket)
            .key(format!("{email}/{name}"))
            .send()
            .await
            .map_err(s3_error)?;
    let ct = mime_guess::from_path(&name)
        .first_or_octet_stream()
        .to_string();
    Ok((
        [(header::CONTENT_TYPE, ct)],
        Body::from_stream(ReaderStream::new(o.body.into_async_read())),
    ))
}
async fn download_system(
    State(a): State<App>,
    h: HeaderMap,
    Path(name): Path<String>,
) -> Api<impl IntoResponse> {
    let _email = auth(&a, &h)?;
    let name = sanitize_system_asset_name(&name)?;
    let o =
        a.s3.get_object()
            .bucket(&a.bucket)
            .key(format!("system/{name}"))
            .send()
            .await
            .map_err(s3_error)?;
    let ct = mime_guess::from_path(&name)
        .first_or_octet_stream()
        .to_string();
    Ok((
        [(header::CONTENT_TYPE, ct)],
        Body::from_stream(ReaderStream::new(o.body.into_async_read())),
    ))
}
async fn delete_file(
    State(a): State<App>,
    h: HeaderMap,
    Path(name): Path<String>,
) -> Api<impl IntoResponse> {
    let email = auth(&a, &h)?;
    let name = sanitize_file_name(&name)?;
    let key = format!("{email}/{name}");
    let size = match a.s3.head_object().bucket(&a.bucket).key(&key).send().await {
        Ok(resp) => resp.content_length().unwrap_or(0),
        Err(_) => return Err((StatusCode::NOT_FOUND, "file not found".into())),
    };
    reserve_storage(&a, &email, -size).await?;
    let delete_result = a.s3.delete_object().bucket(&a.bucket).key(&key).send().await;
    if let Err(err) = delete_result {
        let _ = reserve_storage(&a, &email, size).await;
        return Err(e(err));
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn vm_snapshot(State(a): State<App>, h: HeaderMap) -> Api<impl IntoResponse> {
    let email = auth(&a, &h)?;
    ensure_user_snapshot(&a, &email).await?;
    let key = format!("{email}/{JIT_OS_SNAPSHOT}");
    let o =
        a.s3.get_object()
            .bucket(&a.bucket)
            .key(key)
            .send()
            .await
            .map_err(s3_error)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    if let Some(len) = o.content_length()
        && let Ok(value) = HeaderValue::from_str(&len.to_string())
    {
        headers.insert(header::CONTENT_LENGTH, value);
    }
    Ok((
        headers,
        Body::from_stream(ReaderStream::new(o.body.into_async_read())),
    ))
}

async fn save_vm_snapshot(State(a): State<App>, h: HeaderMap, b: Body) -> Api<impl IntoResponse> {
    let email = auth(&a, &h)?;
    let mut put =
        a.s3.put_object()
            .bucket(&a.bucket)
            .key(format!("{email}/{JIT_OS_SNAPSHOT}"))
            .body(byte_stream_from_axum_body(b));
    if let Ok(len) = content_length(&h) {
        put = put.content_length(len);
    }
    put.send().await.map_err(e)?;
    Ok(StatusCode::CREATED)
}

async fn ensure_user_snapshot(a: &App, email: &str) -> Api<()> {
    let key = format!("{email}/{JIT_OS_SNAPSHOT}");
    if a.s3
        .head_object()
        .bucket(&a.bucket)
        .key(&key)
        .send()
        .await
        .is_ok()
    {
        return Ok(());
    }

    if a.s3
        .head_object()
        .bucket(&a.bucket)
        .key(SYSTEM_JIT_OS_SNAPSHOT)
        .send()
        .await
        .is_ok()
    {
        a.s3.copy_object()
            .bucket(&a.bucket)
            .key(key)
            .copy_source(format!("{}/{}", a.bucket, SYSTEM_JIT_OS_SNAPSHOT))
            .send()
            .await
            .map_err(e)?;
    } else {
        a.s3.put_object()
            .bucket(&a.bucket)
            .key(key)
            .content_length(0)
            .body(ByteStream::from(Vec::new()))
            .send()
            .await
            .map_err(e)?;
    }
    Ok(())
}

async fn upgrade(State(a): State<App>, h: HeaderMap) -> Api<impl IntoResponse> {
    let email = auth(&a, &h)?;
    a.db.update_item()
        .table_name(&a.table)
        .key("email", Av::S(email))
        .update_expression("SET paid = :p")
        .expression_attribute_values(":p", Av::Bool(true))
        .send()
        .await
        .map_err(e)?;
    Ok(StatusCode::OK)
}

fn byte_stream_from_axum_body(body: Body) -> ByteStream {
    let (mut sender, channel_body) = hyper::Body::channel();
    tokio::spawn(async move {
        let mut stream = body.into_data_stream();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    if sender.send_data(bytes).await.is_err() {
                        break;
                    }
                }
                Err(err) => {
                    eprintln!("request body stream error: {err}");
                    break;
                }
            }
        }
    });
    ByteStream::from_body_0_4(channel_body)
}

fn content_length(h: &HeaderMap) -> Api<i64> {
    let raw = h
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .ok_or((
            StatusCode::LENGTH_REQUIRED,
            "Content-Length required".into(),
        ))?;
    raw.parse::<i64>()
        .ok()
        .filter(|n| *n >= 0)
        .ok_or((StatusCode::BAD_REQUEST, "bad Content-Length".into()))
}

fn sanitize_email(input: &str) -> Api<String> {
    let email = input.trim().to_ascii_lowercase();
    if email.len() < 3
        || email.len() > 254
        || !email.contains('@')
        || email.starts_with('@')
        || email.ends_with('@')
        || email.contains('/')
        || email.contains('\\')
        || email.contains("..")
        || !email
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'@' | b'.' | b'_' | b'+' | b'-'))
    {
        return Err((StatusCode::BAD_REQUEST, "bad email".into()));
    }
    Ok(email)
}

fn sanitize_system_asset_name(input: &str) -> Api<String> {
    let name = input.trim();
    let ext = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or_default();
    if name.is_empty()
        || name.len() > 128
        || name == "."
        || name == ".."
        || name.contains("..")
        || name.contains('/')
        || name.contains('\\')
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
        || !matches!(ext, "ext2" | "img" | "raw" | "bin" | "wasm" | "js" | "json")
    {
        return Err((StatusCode::BAD_REQUEST, "bad system asset name".into()));
    }
    Ok(name.to_string())
}

fn sanitize_file_name(input: &str) -> Api<String> {
    let name = input.trim();
    let lower = name.to_ascii_lowercase();
    let first_token = lower
        .split(['.', '-', '_'])
        .next()
        .unwrap_or_default();

    if name.is_empty()
        || name.len() > 128
        || name == "."
        || name == ".."
        || name.starts_with('.')
        || name.ends_with('.')
        || name.contains("..")
        || first_token == "snapshots"
        || first_token == "system"
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
        || !name.bytes().any(|b| b.is_ascii_alphanumeric())
    {
        return Err((StatusCode::BAD_REQUEST, "bad name".into()));
    }

    if let Some((stem, ext)) = name.rsplit_once('.')
        && (stem.is_empty()
            || ext.is_empty()
            || ext.len() > 16
            || !ext.bytes().all(|b| b.is_ascii_alphanumeric()))
    {
        return Err((StatusCode::BAD_REQUEST, "bad extension".into()));
    }

    Ok(name.to_string())
}

async fn reserve_storage(a: &App, email: &str, delta: i64) -> Api<()> {
    if delta == 0 {
        return Ok(());
    }

    let mut req = a
        .db
        .update_item()
        .table_name(&a.table)
        .key("email", Av::S(email.to_string()))
        .update_expression("SET #used = if_not_exists(#used, :zero) + :delta")
        .expression_attribute_names("#used", "used")
        .expression_attribute_values(":zero", Av::N("0".into()))
        .expression_attribute_values(":delta", Av::N(delta.to_string()));

    if delta > 0 {
        let max_used = a.free - delta;
        req = req
            .condition_expression("#paid = :paid OR attribute_not_exists(#used) OR #used <= :max_used")
            .expression_attribute_names("#paid", "paid")
            .expression_attribute_values(":paid", Av::Bool(true))
            .expression_attribute_values(":max_used", Av::N(max_used.to_string()));
    } else {
        req = req
            .condition_expression("#used >= :refund")
            .expression_attribute_values(":refund", Av::N((-delta).to_string()));
    }

    match req.send().await {
        Ok(_) => Ok(()),
        Err(err) => {
            let err_s = err.to_string();
            if delta > 0 && err_s.contains("ConditionalCheckFailed") {
                Err((StatusCode::PAYMENT_REQUIRED, "storage limit reached".into()))
            } else if delta < 0 && err_s.contains("ConditionalCheckFailed") {
                Err((StatusCode::CONFLICT, "storage balance update conflict".into()))
            } else {
                Err(e(err))
            }
        }
    }
}

fn auth(a: &App, h: &HeaderMap) -> Api<String> {
    h.get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|c| c.split(';').find_map(|p| p.trim().strip_prefix("sid=")))
        .ok_or((StatusCode::UNAUTHORIZED, "login required".into()))
        .and_then(|t| {
            decode::<Claims>(
                t,
                &DecodingKey::from_secret(a.jwt.as_bytes()),
                &Validation::default(),
            )
            .map(|d| d.claims.sub)
            .map_err(e)
        })
}
async fn user(a: &App, email: &str) -> Api<HashMap<String, Av>> {
    a.db.get_item()
        .table_name(&a.table)
        .key("email", Av::S(email.into()))
        .send()
        .await
        .map_err(e)?
        .item
        .ok_or((StatusCode::UNAUTHORIZED, "not found".into()))
}
fn num(u: &HashMap<String, Av>, k: &str) -> i64 {
    u.get(k)
        .and_then(|v| v.as_n().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}
fn paid(u: &HashMap<String, Av>) -> bool {
    u.get("paid")
        .and_then(|v| v.as_bool().ok())
        .copied()
        .unwrap_or(false)
}
fn s3_error<E: std::fmt::Debug + std::fmt::Display>(err: E) -> (StatusCode, String) {
    let debug = format!("{err:?}");
    if debug.contains("NoSuchKey")
        || debug.contains("NotFound")
        || debug.contains("StatusCode(404)")
    {
        (StatusCode::NOT_FOUND, "S3 object not found".into())
    } else {
        e(err)
    }
}

fn e<E: std::fmt::Debug + std::fmt::Display>(e: E) -> (StatusCode, String) {
    eprintln!("API ERROR debug: {:?}", e);
    eprintln!("API ERROR display: {}", e);
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

async fn vm_net(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|socket| async move {
        let mut relay = match tokio::net::TcpStream::connect("relay.widgetry.org:80").await {
            Ok(s) => s,
            Err(_) => return,
        };
        let (mut ws_sender, mut ws_receiver) = socket.split();
        let (mut tcp_reader, mut tcp_writer) = relay.split();

        let ws_to_tcp = async {
            use tokio::io::AsyncWriteExt;
            while let Some(Ok(msg)) = ws_receiver.next().await {
                if let Message::Binary(data) = msg
                    && tcp_writer.write_all(&data).await.is_err()
                {
                    break;
                }
            }
        };

        let tcp_to_ws = async {
            use tokio::io::AsyncReadExt;
            let mut buf = [0u8; 16384];
            while let Ok(n) = tcp_reader.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                if ws_sender
                    .send(Message::Binary(buf[..n].to_vec().into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        };

        tokio::join!(ws_to_tcp, tcp_to_ws);
    })
}
