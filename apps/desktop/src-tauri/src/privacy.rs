//! Keeps server addresses out of the UI: the app may be distributed to third parties and the
//! server's address must not be shown (user requirement). Full details go to the local log only.

use std::sync::LazyLock;

use regex::Regex;

static URL: LazyLock<Regex> = LazyLock::new(|| {
    // Trailing punctuation (": refused", "...,") is not part of the address.
    Regex::new(r#"(?i)\b(?:wss?|https?|turns?|stuns?):(?://)?[^\s"'()<>]*[^\s"'()<>:.,;]"#)
        .expect("valid regex")
});
static SSLIP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:\d{1,3}-){3}\d{1,3}\.sslip\.io\b").expect("valid regex")
});
static WORKERS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[a-z0-9-]+(?:\.[a-z0-9-]+)*\.workers\.dev\b").expect("valid regex")
});
static IPV4: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\d{1,3}(?:\.\d{1,3}){3}(?::\d{1,5})?\b").expect("valid regex"));

/// Removes URLs, IPv4 addresses and IP-bearing host names from a message meant for the UI.
pub fn redact(message: &str) -> String {
    let s = URL.replace_all(message, "[server]");
    let s = SSLIP.replace_all(&s, "[server]");
    let s = WORKERS.replace_all(&s, "[server]");
    IPV4.replace_all(&s, "[indirizzo]").into_owned()
}

/// Turns a failure to reach the signaling server into a short message without addresses.
pub fn connect_error(error: &anyhow::Error) -> String {
    let text = format!("{error:#}").to_lowercase();
    let reason = if text.contains("connection refused") {
        "il server rifiuta la connessione (manutenzione in corso?)"
    } else if text.contains("network is unreachable") || text.contains("no route to host") {
        "nessuna connessione di rete"
    } else if text.contains("dns")
        || text.contains("lookup")
        || text.contains("nodename")
        || text.contains("name or service not known")
        || text.contains("no such host")
    {
        "nessuna connessione a Internet o server non trovato"
    } else if text.contains("timed out") || text.contains("timeout") {
        "il server non risponde"
    } else if text.contains("tls") || text.contains("certificate") {
        "connessione sicura al server non riuscita"
    } else {
        "impossibile collegarsi al server"
    };
    reason.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_urls_hosts_and_ips() {
        let msg = "connecting to signaling server wss://203-0-113-10.sslip.io/ws: refused";
        assert_eq!(
            redact(msg),
            "connecting to signaling server [server]: refused"
        );
        assert_eq!(
            redact("relay turn:203.0.113.10:3478?transport=udp"),
            "relay [server]"
        );
        assert_eq!(
            redact("host 203.0.113.10:3478 down"),
            "host [indirizzo] down"
        );
        assert_eq!(redact("via 203-0-113-10.sslip.io"), "via [server]");
        assert_eq!(
            redact("lookup kronos-signaling.kronos-signaling.workers.dev failed"),
            "lookup [server] failed"
        );
        assert_eq!(redact("ID 123 456 789 connesso"), "ID 123 456 789 connesso");
    }

    #[test]
    fn friendly_connect_errors() {
        let refused = anyhow::anyhow!("IO error: Connection refused (os error 61)")
            .context("connecting to signaling server wss://198.51.100.1/ws");
        assert_eq!(
            connect_error(&refused),
            "il server rifiuta la connessione (manutenzione in corso?)"
        );
        let other = anyhow::anyhow!("something odd at 198.51.100.1");
        assert_eq!(connect_error(&other), "impossibile collegarsi al server");
    }
}
