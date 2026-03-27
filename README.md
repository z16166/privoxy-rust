# Privoxy-Rust

A Rust (2024 Edition) reimplementation of [Privoxy](https://www.privoxy.org/), the non-caching web proxy with advanced filtering capabilities for enhancing privacy, modifying web page data and HTTP headers, controlling access, and removing ads and other obnoxious Internet junk.

## Overview

Privoxy-Rust is a feature-equivalent port of the original C-based Privoxy proxy server. It is fully compatible with the original Privoxy configuration files, action files, and filter files. 

> [!IMPORTANT]
> This repository contains the **refactored Privoxy main executable only**. All required configuration files (such as `config.txt`, `default.action`, `default.filter`, etc.) should be extracted from the official [Privoxy installation package](https://www.privoxy.org/).

Key advantages over the C implementation:
- **Memory safety** through Rust's ownership system
- **Async I/O** via Tokio for high-performance networking
- **Type-safe error handling** with `Result` types
- **Thread safety** enforced by Rust's type system

## Building

### Prerequisites

- Rust 1.85+ (for 2024 Edition support)
- On Windows: MSVC build tools

### Build Commands

```bash
# Default build (includes most features)
cargo build --release

# Build with tray icon (Windows GUI with system tray)
cargo build --release --features tray-icon

# Build with Windows service support
cargo build --release --features windows-service

# Build with all common features
cargo build --release --features "tray-icon,cgi,compression,https-inspection"

# Run tests
cargo test
```

## Features

Features are configured in `Cargo.toml` and toggled via `--features` flags at build time.

### Default Features

The following features are enabled by default:

| Feature | Description |
|---|---|
| `rustls` | TLS/SSL backend using rustls (pure Rust) |
| `compression` | HTTP response compression/decompression (gzip, deflate) via `flate2` |
| `toggle` | Runtime enable/disable toggle for Privoxy filtering |
| `force-load` | Force-load support for bypassing filters |
| `fast-redirects` | Detect and follow fast redirects |
| `statistics` | Track request/connection statistics |
| `image-blocking` | Replace blocked images with a pattern or transparent GIF |
| `acl` | IP-based access control lists |
| `trust` | Trust-based access control |
| `cgi-edit-actions` | Web-based action file editor |
| `connection-keep-alive` | HTTP Keep-Alive connection reuse |
| `connection-sharing` | Share upstream connections across clients |
| `client-tags` | Client-specific tag-based filtering |

### Optional Features

| Feature | Description |
|---|---|
| **SSL/TLS Backends** | |
| `openssl-ssl` | TLS via OpenSSL (vendored) |
| `native-tls` | TLS via platform-native library |
| `rustls` | TLS via rustls (default) |
| `mbedtls` | TLS via mbedTLS |
| `wolfssl` | TLS via wolfSSL |
| **HTTPS Inspection** | |
| `https-inspection` | HTTPS traffic inspection (MITM) using rustls |
| `https-inspection-mbedtls` | HTTPS inspection using mbedTLS |
| `https-inspection-openssl` | HTTPS inspection using OpenSSL |
| `https-inspection-wolfssl` | HTTPS inspection using wolfSSL |
| **Platform Features** | |
| `tray-icon` | System tray icon with menu (Windows; uses `tray-icon`, `winit`, `libui`) |
| `windows-service` | Run as a Windows service |
| **Other** | |
| `cgi` | Built-in CGI web interface for status and configuration (uses `hyper`) |
| `extended-statistics` | Extended statistics tracking |
| `pcre-host-patterns` | PCRE-style host pattern matching |
| `external-filter` | External filter program support |
| `graceful-termination` | Graceful shutdown support |
| `no-gifs` | Disable built-in GIF images |
| `accept-filter` | TCP accept filter support |

## Usage

```bash
# Run with default configuration
privoxy-rust

# Run with a specific configuration file
privoxy-rust /path/to/config.txt

# Test configuration without starting the server
privoxy-rust --config-test /path/to/config.txt

# Enable verbose logging
privoxy-rust -v

# Show help
privoxy-rust --help
```

## Configuration

The configuration file format is fully compatible with the original Privoxy. 

> [!NOTE]
> All configuration files (`config.txt`, `*.action`, `*.filter`) should be moved from the official Privoxy installation to the program directory before running.

Example configuration:

```
listen-address  127.0.0.1:8118
logfile         privoxy.log
actionsfile     default.action
actionsfile     pac.action
filterfile      default.filter
permit-access   192.168.0.0/16
toggle          1
```

### Forwarding Rules (pac.action)

Forward traffic through upstream SOCKS5/HTTP proxies using action files:

```
# Direct connection (no proxy)
{+forward-override{forward .}}
/

# Forward .youtube.com via SOCKS5 proxy
{+forward-override{forward-socks5 127.0.0.1:9090 .}}
.youtube.com/

# Forward .example.com via HTTP proxy
{+forward-override{forward 192.168.1.1:3128}}
.example.com/
```

## Project Structure

```
privoxy-rust/
├── Cargo.toml             # Package configuration and feature definitions
├── build.rs               # Build script (Windows resource embedding)
├── privoxy.rc             # Windows resource file (icons, version info)
├── app.manifest           # Windows application manifest
├── assets/                # Icon files (privoxy.ico, radar-*.ico, off.ico)
└── src/
    ├── main.rs            # Entry point, CLI parsing, server startup
    ├── server.rs          # TCP listener and connection acceptance
    ├── connection.rs      # Connection handling, SOCKS5/HTTP forwarding, tunneling
    ├── http.rs            # HTTP request/response parsing
    ├── config.rs          # Configuration file parsing
    ├── action.rs          # Action file loading, URL pattern matching, filtering
    ├── filter.rs          # Content filtering engine
    ├── loaders.rs         # Action/filter file loading, forward directive parsing
    ├── cgi.rs             # Built-in CGI web interface
    ├── cgiedit.rs         # CGI action editor
    ├── ssl.rs             # TLS/SSL handling
    ├── compression.rs     # HTTP compression/decompression
    ├── client_tags.rs     # Client tag management
    ├── tray_icon.rs       # System tray icon (Windows)
    ├── windows_service.rs # Windows service support
    ├── deanimate.rs       # GIF deanimation
    ├── encode.rs          # URL/HTML encoding utilities
    ├── errlog.rs          # Error logging
    ├── logger.rs          # Logging infrastructure
    ├── state.rs           # Application state management
    ├── constants.rs       # Constants and version info
    ├── error.rs           # Error types
    ├── feature.rs         # Feature flag utilities
    └── util.rs            # Miscellaneous utilities
```

## License

GNU General Public License v2.0 or later, same as the original Privoxy.

## Original Project

This is a Rust port of [Privoxy](https://www.privoxy.org/), originally written in C by the Privoxy team.
