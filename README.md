# Rust Cloud Terminal

A high-performance, serverless-computation architecture for a browser-based persistent Linux terminal. Built using an Axum API backend, a Yew/WebAssembly frontend, and automated workspace persistence backed by AWS DynamoDB and Amazon S3.

Instead of heavy hardware emulation or expensive server-side compute instances, this project utilizes client-side OS JIT (Just-In-Time) virtualization to translate 64-bit Linux binaries directly into WebAssembly instructions in real-time, delivering near-native execution speeds completely free of server compute costs.

---

## What It Does

* Zero-Compute Infrastructure: Runs a fully functional, unrestricted 64-bit Linux runtime (Debian/Ubuntu) directly inside the client's browser engine.
* High-Velocity Web Terminal: Provides a fullscreen xterm.js interface bound directly to the virtualized client-side OS execution layer.
* Agnostic Development Sandbox: Allows users to execute standard Linux commands, manage packages (apt update && apt install), and run compilers/runtimes (Rust, Go, Python, GCC) dynamically.
* Automated Workspace Persistence: Seamlessly intercepts filesystem shifts, backing up and restoring differential OS overlay snapshots (.bin) automatically via Amazon S3.
* Secure API Core: Features JWT-protected cookie authentication, strict path-traversal sanitization, and atomic storage quota allocations guarded by Amazon DynamoDB condition expressions.

---

## Project Layout

```text
.
├── Cargo.toml          # Workspace manifest
├── Cargo.lock
├── server/             # Axum API server & static asset delivery
└── web/                # Yew WASM frontend dashboard
```

*Large local runtime images, generated build outputs, logs, and state files are intentionally isolated and ignored by Git.*

---

## Prerequisites

* Rust toolchain (2024 edition matching Cargo.toml)
* wasm32-unknown-unknown compilation target
* trunk for asset bundling and frontend pipeline
* AWS credentials configured with adequate DynamoDB and S3 block permissions

rustup target add wasm32-unknown-unknown
cargo install trunk

---

## AWS Setup

### 1. Provision Infrastructure
Create your DynamoDB tracking schema and your dedicated deployment S3 bucket:

aws dynamodb create-table \
  --table-name rust-cloud-users \
  --attribute-definitions AttributeName=email,AttributeType=S \
  --key-schema AttributeName=email,KeyType=HASH \
  --billing-mode PAY_PER_REQUEST

aws s3 mb s3://YOUR_BUCKET

### 2. IAM Policies
Ensure your runtime environment execution role has explicit rights to:
* dynamodb:GetItem, dynamodb:PutItem, dynamodb:UpdateItem on the targeted users table.
* s3:ListBucket, s3:GetObject, s3:PutObject, s3:DeleteObject on your S3 assets (s3:GetObject validates HEAD check tokens).

---

## Build and Run

### 1. Compile Frontend Assets
Bundle the Yew application distribution directly into the server's static routing path:

cd web
trunk build index.html --dist ../server/web-dist --release --no-sri
cd ..

### 2. Launch the Application Server
Export your production environment variables and wake the Axum engine:

export USERS_TABLE=rust-cloud-users
export S3_BUCKET=YOUR_BUCKET
export JWT_SECRET='your-secure-jwt-crypto-secret-key'
export FREE_GB=5

cargo run -p server

Navigate your browser to http://localhost:3000.

---

## Persistence & Workspace Mechanics

On successful signup or login, the Axum core validates user credentials and ensures a baseline system overlay token is active. 

// Configured inside web/index.html to force private asset sourcing
window.RUST_CLOUD_OS_BASE_IMAGE_URL = "/api/system/debian-base.ext2";

### Asset Seed Deployment
Upload your pristine filesystem base and seed snapshots to your cloud environment:

aws s3 cp debian-base.ext2 s3://YOUR_BUCKET/system/debian-base.ext2
aws s3 cp jit-os-overlay.bin s3://YOUR_BUCKET/system/snapshots/jit-os-overlay.bin

The application environment initiates boots completely automatically upon successful dashboard session validation. It streams your unique workspace blocks straight into the browser JIT cache and automatically stages an internal commit back to Amazon S3 on user terminal idle patterns, explicit manual save triggers, or safe session sign-out requests.

---

## 🛠️ Development Methodology

This project was engineered using a high-velocity, Agentic AI coding workflow. The overarching system design, architectural boundaries, and performance-critical mechanisms—such as asynchronous HTTP response body streaming (hyper::Body::channel) to prevent Out-of-Memory (OOM) faults, Cross-Origin Isolation headers for secure SharedArrayBuffer threading, and atomic DynamoDB condition locks—were architected by me, while code synthesis and boilerplate generation were accelerated via advanced AI orchestration tools.
