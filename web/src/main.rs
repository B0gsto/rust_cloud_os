use gloo_net::http::Request;
use serde::{Deserialize, Serialize};
use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlInputElement;
use yew::prelude::*;

#[derive(Deserialize, Clone, Default, Debug, PartialEq)]
struct Me {
    email: String,
    used: i64,
    free: i64,
    paid: bool,
}

#[derive(Serialize)]
struct AuthRequest {
    email: String,
    password: String,
}

fn format_bytes(bytes: i64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1_048_576 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else if bytes < 1_073_741_824 {
        format!("{:.2} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{:.2} GB", bytes as f64 / 1_073_741_824.0)
    }
}

#[derive(Properties, PartialEq)]
struct VirtualTerminalWorkspaceProps {
    is_stopped_or_error: bool,
    is_error: bool,
    curr_msg: String,
    profile_title: String,
    on_boot: Callback<MouseEvent>,
}

#[function_component(VirtualTerminalWorkspace)]
fn virtual_terminal_workspace(props: &VirtualTerminalWorkspaceProps) -> Html {
    html! {
        <div class="dashboard-main" style="grid-template-columns:1fr;">
            <div class="dashboard-content" style="border-radius:0;">
                <div class="tab-content" style="display:flex;">
                    <div class="vm-desktop-container virtual-terminal-workspace">
                        <div class="terminal-toolbar">
                            <div class="terminal-toolbar-title">
                                <span class="terminal-led"></span>
                                <span>{"Browser JIT Linux Terminal"}</span>
                            </div>
                            <span class="terminal-toolbar-profile">{props.profile_title.clone()}</span>
                        </div>

                        <div class="vm-screen-wrapper terminal-screen-wrapper">
                            <div id="terminal-container" class="terminal-container" aria-label="Linux terminal"></div>

                            { if props.is_stopped_or_error {
                                html! {
                                    <div class="vm-splash terminal-splash">
                                        <div class="vm-splash-icon">{"⌨️"}</div>
                                        <h3>{props.profile_title.clone()}</h3>
                                        <p>{"Start a client-side Linux workspace. The runtime attaches stdin/stdout/stderr directly to xterm.js; saved block-state snapshots are synchronized through the existing authenticated S3 API."}</p>
                                        { if props.is_error { html! { <p class="vm-error-text">{props.curr_msg.clone()}</p> } } else { html! {} }}
                                        <button class="vm-btn vm-btn-boot vm-btn-lg" onclick={props.on_boot.clone()} style="display:flex;align-items:center;gap:8px;padding:12px 30px;font-weight:700;">{"▶ Start Linux"}</button>
                                    </div>
                                }
                            } else { html! {} }}
                        </div>
                    </div>
                </div>
            </div>
        </div>
    }
}

#[function_component(App)]
fn app() -> Html {
    let me = use_state(|| None::<Me>);
    let email = use_state(String::new);
    let password = use_state(String::new);
    let msg = use_state(String::new);
    let msg_is_error = use_state(|| false);
    let auth_tab = use_state(|| "login");

    let vm_profile = use_state(|| String::from("debian_base"));
    let vm_status = use_state(|| String::from("stopped"));
    let vm_status_msg = use_state(String::new);

    let refresh = {
        let me = me.clone();
        Callback::from(move |_| {
            let me = me.clone();
            spawn_local(async move {
                if let Ok(r) = Request::get("/api/me").send().await {
                    if r.ok() {
                        me.set(r.json::<Option<Me>>().await.unwrap_or(None));
                    } else {
                        me.set(None);
                    }
                }
            });
        })
    };

    {
        let refresh = refresh.clone();
        use_effect_with((), move |_| {
            refresh.emit(());
            || ()
        });
    }

    let auth = |route: &'static str| {
        let email = email.clone();
        let password = password.clone();
        let msg = msg.clone();
        let msg_is_error = msg_is_error.clone();
        let refresh = refresh.clone();
        Callback::from(move |_| {
            let email_str = (*email).clone();
            let password_str = (*password).clone();
            if email_str.is_empty() || password_str.is_empty() {
                msg.set("Email and password are required".into());
                msg_is_error.set(true);
                return;
            }

            let body = AuthRequest {
                email: email_str,
                password: password_str,
            };
            let msg = msg.clone();
            let msg_is_error = msg_is_error.clone();
            let refresh = refresh.clone();
            let email = email.clone();
            let password = password.clone();
            spawn_local(async move {
                match Request::post(route).json(&body).unwrap().send().await {
                    Ok(r) if r.ok() => {
                        if route == "/api/signup" {
                            msg.set("Account created. Sign in to start Linux.".into());
                        } else {
                            msg.set(String::new());
                            email.set(String::new());
                            password.set(String::new());
                        }
                        msg_is_error.set(false);
                        refresh.emit(());
                    }
                    Ok(r) => {
                        msg.set(r.text().await.unwrap_or_default());
                        msg_is_error.set(true);
                    }
                    Err(e) => {
                        msg.set(e.to_string());
                        msg_is_error.set(true);
                    }
                }
            });
        })
    };

    let on_logout = {
        let me = me.clone();
        let msg = msg.clone();
        let msg_is_error = msg_is_error.clone();
        let vm_status = vm_status.clone();
        let vm_status_msg = vm_status_msg.clone();
        Callback::from(move |_| {
            let me = me.clone();
            let msg = msg.clone();
            let msg_is_error = msg_is_error.clone();
            let vm_status = vm_status.clone();
            let vm_status_msg = vm_status_msg.clone();
            spawn_local(async move {
                let _ = js_sys::eval("if (window.stopLinuxVM) window.stopLinuxVM();");
                if let Ok(r) = Request::post("/api/logout").send().await
                    && r.ok()
                {
                    me.set(None);
                    vm_status.set("stopped".into());
                    vm_status_msg.set(String::new());
                    msg.set("Logged out.".into());
                    msg_is_error.set(false);
                }
            });
        })
    };

    let boot_vm = {
        let vm_profile = vm_profile.clone();
        let vm_status = vm_status.clone();
        let vm_status_msg = vm_status_msg.clone();
        Callback::from(move |_| {
            let profile = (*vm_profile).clone();
            let status = vm_status.clone();
            let status_msg = vm_status_msg.clone();
            status.set("downloading".into());
            status_msg.set("Preparing browser runtime...".into());

            let status_cb = move |new_status: String, message: String| {
                status.set(new_status);
                status_msg.set(message);
            };
            let status_closure =
                wasm_bindgen::prelude::Closure::<dyn FnMut(String, String)>::new(status_cb);
            let _ = js_sys::Reflect::set(
                &web_sys::window().unwrap(),
                &"onVMStatus".into(),
                status_closure.as_ref(),
            );
            status_closure.forget();

            let _ = js_sys::eval(&format!(
                "if (window.bootLinuxVM) window.bootLinuxVM({:?}, window.onVMStatus);",
                profile
            ));
        })
    };

    let stop_vm = {
        let vm_status = vm_status.clone();
        let vm_status_msg = vm_status_msg.clone();
        Callback::from(move |_| {
            let _ = js_sys::eval("if (window.stopLinuxVM) window.stopLinuxVM();");
            vm_status.set("stopped".into());
            vm_status_msg.set(String::new());
        })
    };

    let save_snapshot = {
        let msg = msg.clone();
        let msg_is_error = msg_is_error.clone();
        Callback::from(move |_| {
            let ok_msg = msg.clone();
            let ok_err = msg_is_error.clone();
            let on_done = wasm_bindgen::prelude::Closure::<dyn Fn()>::new(move || {
                ok_msg.set("Saved Linux workspace snapshot to S3.".into());
                ok_err.set(false);
            });
            let err_msg = msg.clone();
            let err_err = msg_is_error.clone();
            let on_error = wasm_bindgen::prelude::Closure::<dyn Fn(String)>::new(move |e| {
                err_msg.set(format!("Snapshot save failed: {}", e));
                err_err.set(true);
            });
            let _ = js_sys::Reflect::set(
                &web_sys::window().unwrap(),
                &"onSnapshotSaved".into(),
                on_done.as_ref(),
            );
            let _ = js_sys::Reflect::set(
                &web_sys::window().unwrap(),
                &"onSnapshotError".into(),
                on_error.as_ref(),
            );
            on_done.forget();
            on_error.forget();
            let _ = js_sys::eval(
                "if (window.saveLinuxVMSnapshot) window.saveLinuxVMSnapshot(window.onSnapshotSaved, window.onSnapshotError);",
            );
        })
    };

    let on_email_input = {
        let email = email.clone();
        Callback::from(move |e: InputEvent| {
            let input: HtmlInputElement = e.target_unchecked_into();
            email.set(input.value());
        })
    };
    let on_password_input = {
        let password = password.clone();
        Callback::from(move |e: InputEvent| {
            let input: HtmlInputElement = e.target_unchecked_into();
            password.set(input.value());
        })
    };
    let on_profile_change = {
        let vm_profile = vm_profile.clone();
        Callback::from(move |e: Event| {
            let input: HtmlInputElement = e.target_unchecked_into();
            vm_profile.set(input.value());
        })
    };

    let tab = *auth_tab;
    let is_err = *msg_is_error;
    let msg_val = &*msg;
    let curr_status = (*vm_status).clone();
    let curr_msg = (*vm_status_msg).clone();
    let profile_title = match (*vm_profile).as_str() {
        "custom_workspace" => "Custom Saved Workspace",
        _ => "Debian Base Shell",
    };
    let status_text = match curr_status.as_str() {
        "running" => "Running",
        "booting" => "Booting...",
        "downloading" => "Preparing...",
        "error" => "Error",
        _ => "Stopped",
    };
    let badge_class = format!("vm-status-badge-lg {}", curr_status);

    html! {
        <>
            { if let Some(m) = &*me {
                html! {
                    <div class="dashboard-container">
                        { if !msg_val.is_empty() {
                            html! { <div class={if is_err { "msg-error" } else { "msg-success" }} style="position: fixed; top: 75px; right: 20px; z-index: 10001; min-width: 280px; box-shadow: 0 10px 30px rgba(0,0,0,0.5);">{msg_val}</div> }
                        } else { html! {} }}

                        <div class="dashboard-header">
                            <div class="dashboard-logo-section">
                                <div class="dashboard-logo-indicator"></div>
                                <span class="dashboard-logo">{"Rust Cloud OS"}</span>
                            </div>

                            <div class="dashboard-status-center">
                                { if *vm_status == "stopped" || *vm_status == "error" {
                                    html! {
                                        <div style="display:flex;align-items:center;gap:8px;">
                                            <span style="font-size:11px;opacity:0.7;font-weight:600;text-transform:uppercase;">{"Profile:"}</span>
                                            <select class="vm-distro-dropdown" value={(*vm_profile).clone()} onchange={on_profile_change}>
                                                <option value="debian_base">{"Debian Base Shell"}</option>
                                                <option value="custom_workspace">{"Custom Saved Workspace"}</option>
                                            </select>
                                        </div>
                                    }
                                } else { html! { <span style="font-size:12px;opacity:0.75;font-weight:700;">{profile_title}</span> } }}

                                <div class={badge_class}>
                                    { if curr_status != "stopped" && curr_status != "error" { html! { <div class="status-dot-pulse"></div> } } else { html! {} }}
                                    <span>{status_text}</span>
                                    { if curr_status == "downloading" || curr_status == "booting" { html! { <span style="font-size:10px;opacity:0.8;font-weight:normal;margin-left:4px;">{format!("({})", curr_msg)}</span> } } else { html! {} }}
                                </div>
                            </div>

                            <div class="dashboard-meta-right">
                                { if *vm_status == "stopped" || *vm_status == "error" {
                                    html! { <button class="power-btn boot" onclick={boot_vm.clone()} title="Start Linux">{"▶"}</button> }
                                } else if *vm_status == "running" {
                                    html! {
                                        <>
                                            <button class="power-btn boot" onclick={save_snapshot.clone()} title="Save workspace snapshot to S3">{"💾"}</button>
                                            <button class="power-btn stop" onclick={stop_vm} title="Power Off Runtime">{"■"}</button>
                                        </>
                                    }
                                } else { html! {} }}
                                <span style="font-size:12px;opacity:0.8;font-weight:500;">{&m.email}</span>
                                <span onclick={on_logout} class="btn-signout" style="cursor:pointer;padding:6px 12px;border-radius:8px;background:rgba(239,68,68,0.1);border:1px solid rgba(239,68,68,0.2);color:#f87171;font-size:12px;font-weight:700;transition:var(--transition);">{"Sign Out"}</span>
                            </div>
                        </div>

                        <VirtualTerminalWorkspace
                            is_stopped_or_error={*vm_status == "stopped" || *vm_status == "error"}
                            is_error={*vm_status == "error"}
                            curr_msg={curr_msg.clone()}
                            profile_title={profile_title.to_string()}
                            on_boot={boot_vm.clone()}
                        />

                        <div style="position:fixed;bottom:12px;right:18px;font-size:11px;opacity:0.65;">
                            {format!("S3 storage: {} / {}", format_bytes(m.used), format_bytes(m.free))}
                        </div>
                    </div>
                }
            } else {
                html! {
                    <main class="landing">
                        <section class="hero">
                            <h1>{"Rust Cloud OS"}</h1>
                            <p>{"A high-performance Linux workspace in your browser: xterm.js frontend, client-side JIT runtime, and S3-backed snapshot persistence."}</p>
                            <a href="#auth" class="btn-primary">{"Open Linux"}</a>
                        </section>

                        <div id="auth" class="card-container" style="max-width:480px;margin:0 auto 80px;">
                            <div class="card">
                                <h2 class="card-title">{"Get Started"}</h2>
                                <div class="auth-tabs">
                                    <button class={if tab == "login" { "auth-tab active" } else { "auth-tab" }} onclick={let auth_tab=auth_tab.clone(); let msg=msg.clone(); move |_| { auth_tab.set("login"); msg.set(String::new()); }}>{"Sign In"}</button>
                                    <button class={if tab == "signup" { "auth-tab active" } else { "auth-tab" }} onclick={let auth_tab=auth_tab.clone(); let msg=msg.clone(); move |_| { auth_tab.set("signup"); msg.set(String::new()); }}>{"Sign Up"}</button>
                                </div>
                                <div class="auth-form">
                                    <div class="input-group">
                                        <label class="input-label">{"Email Address"}</label>
                                        <input placeholder="you@example.com" type="email" value={(*email).clone()} oninput={on_email_input}/>
                                    </div>
                                    <div class="input-group">
                                        <label class="input-label">{"Password"}</label>
                                        <input placeholder="••••••••" type="password" value={(*password).clone()} oninput={on_password_input}/>
                                    </div>
                                    <div class="auth-buttons">
                                        { if tab == "signup" { html! { <button class="btn-submit" onclick={auth("/api/signup")}>{"Create Account"}</button> } } else { html! { <button class="btn-submit" onclick={auth("/api/login")}>{"Access Linux"}</button> } }}
                                    </div>
                                </div>
                                { if !msg_val.is_empty() { html! { <div class={if is_err { "msg-error" } else { "msg-info" }}>{msg_val}</div> } } else { html! {} }}
                            </div>
                        </div>
                    </main>
                }
            }}
        </>
    }
}

fn main() {
    yew::Renderer::<App>::new().render();
}
