//! Сведения об устройстве для панелей с лимитом устройств (3x-ui, Remnawave и др.):
//! те же заголовки, что шлёт Happ, — `X-HWID`, `X-Device-OS`, `X-Ver-OS`, `X-Device-Model`.
//! Панель привязывает подписку к первым N устройствам и показывает их владельцу.
//!
//! ID компьютера наружу не уходит: `X-HWID` — хеш от него и адреса сервера. Для одного
//! сервера он постоянный, а разные серверы не могут сопоставить, что это один компьютер.

use crate::sys;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

struct Device {
    machine: Option<Vec<u8>>,
    os: String,
    os_version: String,
    model: String,
}

fn device() -> &'static Device {
    static DEVICE: OnceLock<Device> = OnceLock::new();
    DEVICE.get_or_init(|| {
        let (os, os_version) = sys::os_short();
        Device {
            machine: sys::machine_id(),
            os,
            os_version,
            model: sys::device_name().map(|n| header_safe(&n)).filter(|n| !n.is_empty()).unwrap_or_else(|| "PC".into()),
        }
    })
}

/// Заголовки для запроса подписки по этой ссылке. Без ID компьютера — пусто:
/// подписку без лимита панель отдаст и так, а придумывать ID на ходу нельзя —
/// он будет каждый раз новым и займёт все места.
pub fn headers(url: &str) -> Vec<(&'static str, String)> {
    let d = device();
    let Some(machine) = &d.machine else { return Vec::new() };
    vec![
        ("X-HWID", hwid_for(machine, &server_of(url))),
        ("X-Device-OS", d.os.clone()),
        ("X-Ver-OS", header_safe(&d.os_version)),
        ("X-Device-Model", d.model.clone()),
    ]
}

fn hwid_for(machine: &[u8], server: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"kl!ck hwid v1\0");
    h.update(machine);
    h.update(b"\0");
    h.update(server.as_bytes());
    h.finalize()[..16].iter().map(|b| format!("{b:02x}")).collect()
}

/// Хост сервера подписки — без пути и токена, чтобы смена ссылки на том же сервере не меняла ID.
fn server_of(url: &str) -> String {
    reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_ascii_lowercase)).unwrap_or_default()
}

/// В заголовке — только видимый ASCII: имя ПК кириллицей заголовок не пропустит.
fn header_safe(s: &str) -> String {
    let out: String = s.chars().filter(|c| c.is_ascii_graphic() || *c == ' ').collect();
    out.trim().chars().take(64).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hwid_stable_per_server() {
        let m = b"4c4c4544-0000-1111-2222-333344445555";
        let a = hwid_for(m, &server_of("https://vpn.example:2096/sub/abc"));
        assert_eq!(a, hwid_for(m, &server_of("https://VPN.example:2096/sub/other-token")));
        assert_ne!(a, hwid_for(m, &server_of("https://another.example/sub/abc")));
        assert_ne!(a, hwid_for(b"other-machine", &server_of("https://vpn.example:2096/sub/abc")));
        assert_eq!(a.len(), 32);
    }

    #[test]
    fn header_safe_drops_non_ascii() {
        assert_eq!(header_safe("ВАСЯ-PC"), "-PC");
        assert_eq!(header_safe("  vbu00-pc \n"), "vbu00-pc");
        assert_eq!(header_safe("Компьютер"), "");
    }

    /// Что увидит панель от этого компьютера:
    /// `cargo test -p klick-service hwid::tests::print -- --ignored --nocapture -- <ссылка>`
    #[test]
    #[ignore]
    fn print() {
        let url = std::env::args().last().filter(|a| a.starts_with("http")).unwrap_or_else(|| "https://vpn.example/sub/x".into());
        for (k, v) in headers(&url) {
            println!("{k}: {v}");
        }
    }

    #[test]
    fn headers_present_on_this_machine() {
        let h = headers("https://vpn.example/sub/x");
        if sys::machine_id().is_some() {
            let hwid = &h.iter().find(|(k, _)| *k == "X-HWID").unwrap().1;
            assert!(hwid.len() >= 6);
            assert!(h.iter().all(|(_, v)| reqwest::header::HeaderValue::from_str(v).is_ok()));
        }
    }
}
