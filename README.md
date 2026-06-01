# Rust Cloud Terminal

A Rust workspace for a browser-based persistent Linux terminal: an Axum API server, a Yew/WebAssembly frontend, and authenticated persistence backed by AWS DynamoDB and S3.

## What it does

- Serves a Rust/Yew terminal-only web UI compiled to WASM.
- Provides one fullscreen xterm.js Linux terminal backed by the browser runtime.
- Provides authentication and JWT-protected API routes from an Axum server.
- Stores users in DynamoDB.
- Restores and saves each user's runtime snapshot overlay in S3.
- Includes a free-tier storage limit with a simple `paid=true` DynamoDB override.

## Project layout

```text
.
├── Cargo.toml          # Workspace manifest
├── Cargo.lock
├── server/             # Axum API and static file server
└── web/                # Yew WASM frontend
```

Large local runtime images, generated build output, logs, and process files are intentionally ignored by Git.

## Prerequisites

- Rust toolchain with the 2024 edition
- `wasm32-unknown-unknown` target
- `trunk` for building the frontend
- AWS credentials with DynamoDB and S3 access

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk
```

## AWS setup

Create a DynamoDB users table and an S3 bucket:

```sh
aws dynamodb create-table \
  --table-name rust-cloud-users \
  --attribute-definitions AttributeName=email,AttributeType=S \
  --key-schema AttributeName=email,KeyType=HASH \
  --billing-mode PAY_PER_REQUEST

aws s3 mb s3://YOUR_BUCKET
```

Give the server IAM access to:

- `dynamodb:GetItem`, `dynamodb:PutItem`, `dynamodb:UpdateItem` on the users table
- `s3:ListBucket` on the bucket
- `s3:GetObject`, `s3:PutObject`, `s3:DeleteObject` on bucket objects

The server also uses S3 HEAD requests, which are authorized by `s3:GetObject`.

## Build and run

Build the web frontend into the server's static asset directory:

```sh
cd web
trunk build index.html --dist ../server/web-dist --release --no-sri
cd ..
```

Run the server:

```sh
export USERS_TABLE=rust-cloud-users
export S3_BUCKET=YOUR_BUCKET
export JWT_SECRET='change-me'
export FREE_GB=5

cargo run -p server
```

Open <http://localhost:3000>.

## Optional private base image

By default, the development CheerpX/WebVM fallback opens the public WebVM cloud image. To force a private S3-hosted base image, set this before the bridge in `web/index.html`:

```js
window.RUST_CLOUD_OS_BASE_IMAGE_URL = "/api/system/debian-base.ext2";
```

Then upload the base image and optional seed overlay:

```sh
aws s3 cp debian-base.ext2 s3://YOUR_BUCKET/system/debian-base.ext2
aws s3 cp jit-os-overlay.bin s3://YOUR_BUCKET/system/snapshots/jit-os-overlay.bin
```

On signup and login, the server ensures a per-user object exists at `snapshots/jit-os-overlay.bin`. The terminal boots the persistent workspace automatically, restores that object from S3, and saves it back to S3 automatically after terminal activity, when you click save, and before sign out.

## Paywall behavior

File uploads and snapshot saves that need additional storage are blocked with HTTP `402 Payment Required` when the account exceeds `FREE_GB`, unless the DynamoDB user item has `paid=true`.
