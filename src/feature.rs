//! Feature flags for Privoxy
//! These are controlled via Cargo features, similar to how autoconf controls C preprocessor macros

// SSL/TLS backends
#[cfg(feature = "openssl-ssl")]
pub const FEATURE_HTTPS_INSPECTION_OPENSSL: bool = true;
#[cfg(not(feature = "openssl-ssl"))]
pub const FEATURE_HTTPS_INSPECTION_OPENSSL: bool = false;

#[cfg(feature = "rustls")]
pub const FEATURE_HTTPS_INSPECTION_RUSTLS: bool = true;
#[cfg(not(feature = "rustls"))]
pub const FEATURE_HTTPS_INSPECTION_RUSTLS: bool = false;

#[cfg(feature = "mbedtls")]
pub const FEATURE_HTTPS_INSPECTION_MBEDTLS: bool = true;
#[cfg(not(feature = "mbedtls"))]
pub const FEATURE_HTTPS_INSPECTION_MBEDTLS: bool = false;

#[cfg(feature = "wolfssl")]
pub const FEATURE_HTTPS_INSPECTION_WOLFSSL: bool = true;
#[cfg(not(feature = "wolfssl"))]
pub const FEATURE_HTTPS_INSPECTION_WOLFSSL: bool = false;

#[cfg(feature = "https-inspection")]
pub const FEATURE_HTTPS_INSPECTION: bool = true;
#[cfg(not(feature = "https-inspection"))]
pub const FEATURE_HTTPS_INSPECTION: bool = false;

// Core features
#[cfg(feature = "toggle")]
pub const FEATURE_TOGGLE: bool = true;
#[cfg(not(feature = "toggle"))]
pub const FEATURE_TOGGLE: bool = false;

#[cfg(feature = "force-load")]
pub const FEATURE_FORCE_LOAD: bool = true;
#[cfg(not(feature = "force-load"))]
pub const FEATURE_FORCE_LOAD: bool = false;

#[cfg(feature = "fast-redirects")]
pub const FEATURE_FAST_REDIRECTS: bool = true;
#[cfg(not(feature = "fast-redirects"))]
pub const FEATURE_FAST_REDIRECTS: bool = false;

#[cfg(feature = "statistics")]
pub const FEATURE_STATISTICS: bool = true;
#[cfg(not(feature = "statistics"))]
pub const FEATURE_STATISTICS: bool = false;

#[cfg(feature = "extended-statistics")]
pub const FEATURE_EXTENDED_STATISTICS: bool = true;
#[cfg(not(feature = "extended-statistics"))]
pub const FEATURE_EXTENDED_STATISTICS: bool = false;

#[cfg(feature = "image-blocking")]
pub const FEATURE_IMAGE_BLOCKING: bool = true;
#[cfg(not(feature = "image-blocking"))]
pub const FEATURE_IMAGE_BLOCKING: bool = false;

#[cfg(feature = "acl")]
pub const FEATURE_ACL: bool = true;
#[cfg(not(feature = "acl"))]
pub const FEATURE_ACL: bool = false;

#[cfg(feature = "trust")]
pub const FEATURE_TRUST: bool = true;
#[cfg(not(feature = "trust"))]
pub const FEATURE_TRUST: bool = false;

#[cfg(feature = "cgi-edit-actions")]
pub const FEATURE_CGI_EDIT_ACTIONS: bool = true;
#[cfg(not(feature = "cgi-edit-actions"))]
pub const FEATURE_CGI_EDIT_ACTIONS: bool = false;

#[cfg(feature = "no-gifs")]
pub const FEATURE_NO_GIFS: bool = true;
#[cfg(not(feature = "no-gifs"))]
pub const FEATURE_NO_GIFS: bool = false;

#[cfg(feature = "graceful-termination")]
pub const FEATURE_GRACEFUL_TERMINATION: bool = true;
#[cfg(not(feature = "graceful-termination"))]
pub const FEATURE_GRACEFUL_TERMINATION: bool = false;

#[cfg(feature = "pcre-host-patterns")]
pub const FEATURE_PCRE_HOST_PATTERNS: bool = true;
#[cfg(not(feature = "pcre-host-patterns"))]
pub const FEATURE_PCRE_HOST_PATTERNS: bool = false;

#[cfg(feature = "external-filter")]
pub const FEATURE_EXTERNAL_FILTERS: bool = true;
#[cfg(not(feature = "external-filter"))]
pub const FEATURE_EXTERNAL_FILTERS: bool = false;

#[cfg(feature = "accept-filter")]
pub const FEATURE_ACCEPT_FILTER: bool = true;
#[cfg(not(feature = "accept-filter"))]
pub const FEATURE_ACCEPT_FILTER: bool = false;

#[cfg(feature = "strptime-sanity-checks")]
pub const FEATURE_STRPTIME_SANITY_CHECKS: bool = true;
#[cfg(not(feature = "strptime-sanity-checks"))]
pub const FEATURE_STRPTIME_SANITY_CHECKS: bool = false;

#[cfg(feature = "client-tags")]
pub const FEATURE_CLIENT_TAGS: bool = true;
#[cfg(not(feature = "client-tags"))]
pub const FEATURE_CLIENT_TAGS: bool = false;

#[cfg(feature = "compression")]
pub const FEATURE_COMPRESSION: bool = true;
#[cfg(not(feature = "compression"))]
pub const FEATURE_COMPRESSION: bool = false;

#[cfg(feature = "connection-keep-alive")]
pub const FEATURE_CONNECTION_KEEP_ALIVE: bool = true;
#[cfg(not(feature = "connection-keep-alive"))]
pub const FEATURE_CONNECTION_KEEP_ALIVE: bool = false;

#[cfg(feature = "connection-sharing")]
pub const FEATURE_CONNECTION_SHARING: bool = true;
#[cfg(not(feature = "connection-sharing"))]
pub const FEATURE_CONNECTION_SHARING: bool = false;

/// Check if any HTTPS inspection backend is enabled
pub fn is_https_inspection_enabled() -> bool {
    FEATURE_HTTPS_INSPECTION || 
    FEATURE_HTTPS_INSPECTION_OPENSSL || 
    FEATURE_HTTPS_INSPECTION_RUSTLS || 
    FEATURE_HTTPS_INSPECTION_MBEDTLS || 
    FEATURE_HTTPS_INSPECTION_WOLFSSL
}

/// Get the enabled HTTPS inspection backend
pub fn get_https_inspection_backend() -> Option<&'static str> {
    if FEATURE_HTTPS_INSPECTION_OPENSSL {
        Some("openssl")
    } else if FEATURE_HTTPS_INSPECTION_RUSTLS {
        Some("rustls")
    } else if FEATURE_HTTPS_INSPECTION_MBEDTLS {
        Some("mbedtls")
    } else if FEATURE_HTTPS_INSPECTION_WOLFSSL {
        Some("wolfssl")
    } else {
        None
    }
}
