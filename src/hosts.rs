use std::net::{Ipv4Addr, SocketAddr};

use reqwest::{Url, blocking::Client};

pub const HOSTED_DOMAIN: &str = "fobauth.com";
const HOSTED_CONSOLE: &str = "https://app.fob.so";

#[derive(Debug, Clone, PartialEq)]
pub struct Hosts {
    pub console: String,
    tenant_scheme: &'static str,
    tenant_domain: String,
}

impl Hosts {
    pub fn new(host: &str, console: Option<&str>) -> Self {
        let host = host
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .to_ascii_lowercase();
        let (name, port) = match host.rsplit_once(':') {
            Some((name, port)) => (name, format!(":{port}")),
            None => (host.as_str(), String::new()),
        };
        let (default_console, tenant_scheme, tenant_domain) = if is_local(name) {
            (
                format!("http://app.localhost{port}"),
                "http",
                format!("fobauth.localhost{port}"),
            )
        } else if host == HOSTED_DOMAIN {
            (HOSTED_CONSOLE.to_owned(), "https", host.clone())
        } else {
            (format!("https://app.{host}"), "https", host.clone())
        };
        Self {
            console: console
                .map(|url| url.trim().trim_end_matches('/').to_owned())
                .unwrap_or(default_console),
            tenant_scheme,
            tenant_domain,
        }
    }

    pub fn tenant(&self, org: &str) -> String {
        format!("{}://{org}.{}", self.tenant_scheme, self.tenant_domain)
    }
}

pub fn is_local(name: &str) -> bool {
    name == "localhost" || name.ends_with(".localhost")
}

/// `*.localhost` names only resolve inside browsers on some systems, so they are pinned
/// to loopback for the origins the client will talk to.
pub fn client(origins: &[&str]) -> Client {
    let mut builder = Client::builder()
        .user_agent(concat!("fob-cli/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(60));
    for origin in origins {
        if let Ok(url) = Url::parse(origin)
            && let Some(name) = url.host_str()
            && is_local(name)
        {
            builder = builder.resolve(name, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)));
        }
    }
    builder.build().expect("an HTTP client")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_layout() {
        let hosts = Hosts::new("fobauth.com", None);
        assert_eq!(hosts.console, "https://app.fob.so");
        assert_eq!(hosts.tenant("clyent"), "https://clyent.fobauth.com");
    }

    #[test]
    fn own_domain_layout() {
        let hosts = Hosts::new("https://Example.com/", None);
        assert_eq!(hosts.console, "https://app.example.com");
        assert_eq!(hosts.tenant("clyent"), "https://clyent.example.com");
    }

    #[test]
    fn local_layout() {
        for host in [
            "localhost:8080",
            "app.localhost:8080",
            "fobauth.localhost:8080",
        ] {
            let hosts = Hosts::new(host, None);
            assert_eq!(hosts.console, "http://app.localhost:8080");
            assert_eq!(
                hosts.tenant("clyent"),
                "http://clyent.fobauth.localhost:8080"
            );
        }
        let hosts = Hosts::new("localhost", None);
        assert_eq!(hosts.console, "http://app.localhost");
        assert_eq!(hosts.tenant("clyent"), "http://clyent.fobauth.localhost");
    }

    #[test]
    fn console_override() {
        let hosts = Hosts::new("example.com", Some("https://console.example.com/"));
        assert_eq!(hosts.console, "https://console.example.com");
        assert_eq!(hosts.tenant("clyent"), "https://clyent.example.com");
    }

    #[test]
    fn local_names() {
        assert!(is_local("localhost"));
        assert!(is_local("clyent.fobauth.localhost"));
        assert!(!is_local("localhost.example.com"));
    }
}
