use compact_str::CompactString;
use compact_str::ToCompactString as _;
use core_utils::{ascii_lower_compact, percent_decode, percent_encode_compact};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProxyScheme {
    Http,
    Https,
    Socks5,
    Socks5h,
    Socks4,
}

impl ProxyScheme {
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            ProxyScheme::Http => "http",
            ProxyScheme::Https => "https",
            ProxyScheme::Socks5 => "socks5",
            ProxyScheme::Socks5h => "socks5h",
            ProxyScheme::Socks4 => "socks4",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let lower: CompactString = ascii_lower_compact(s);
        match lower.as_str() {
            "http" => Some(ProxyScheme::Http),
            "https" => Some(ProxyScheme::Https),
            "socks5" => Some(ProxyScheme::Socks5),
            "socks5h" => Some(ProxyScheme::Socks5h),
            "socks4" => Some(ProxyScheme::Socks4),
            _ => None,
        }
    }

    #[inline]
    pub const fn wreq_supported(self) -> bool {
        matches!(
            self,
            ProxyScheme::Http | ProxyScheme::Https | ProxyScheme::Socks5
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TargetAddr {
    Ip(core::net::IpAddr),
    Domain(CompactString),
}

impl TargetAddr {
    #[inline]
    pub const fn is_ipv6(&self) -> bool {
        matches!(self, TargetAddr::Ip(core::net::IpAddr::V6(_)))
    }

    pub fn host_string(&self) -> CompactString {
        match self {
            TargetAddr::Domain(d) => d.clone(),
            TargetAddr::Ip(ip) => ip.to_compact_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProxyAuth {
    pub username: CompactString,
    pub password: CompactString,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProxyConfig {
    pub scheme: ProxyScheme,
    pub target: TargetAddr,
    pub port: u16,
    pub auth: Option<ProxyAuth>,
    pub utc_offset: Option<i32>,
    pub asn: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyParseError {
    Empty,
    TooLong,
    MissingScheme,
    UnsupportedScheme,
    MissingHost,
    MissingPort,
    BadPort,
    BadUserInfo,
}

impl ProxyParseError {
    pub const fn as_str(self) -> &'static str {
        match self {
            ProxyParseError::Empty => "empty proxy url",
            ProxyParseError::TooLong => "proxy url too long",
            ProxyParseError::MissingScheme => "missing scheme",
            ProxyParseError::UnsupportedScheme => "unsupported scheme",
            ProxyParseError::MissingHost => "missing host",
            ProxyParseError::MissingPort => "missing port",
            ProxyParseError::BadPort => "port out of range",
            ProxyParseError::BadUserInfo => "bad user info",
        }
    }
}

impl core::fmt::Display for ProxyParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl core::error::Error for ProxyParseError {}

const MAX_URL: usize = 512;

impl ProxyConfig {
    pub fn parse(s: &str) -> Result<Self, ProxyParseError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(ProxyParseError::Empty);
        }
        if s.len() > MAX_URL {
            return Err(ProxyParseError::TooLong);
        }
        let a = core_utils::url::split_authority(s);
        if a.scheme.is_empty() {
            return Err(ProxyParseError::MissingScheme);
        }
        let scheme = ProxyScheme::parse(a.scheme).ok_or(ProxyParseError::UnsupportedScheme)?;
        let auth = match a.userinfo {
            Some((user, pass)) => {
                let ui_len = user.len() + pass.map_or(0, |p| 1 + p.len());
                if user.is_empty() || ui_len > 256 {
                    return Err(ProxyParseError::BadUserInfo);
                }
                Some(ProxyAuth {
                    username: percent_decode_compact(user)?,
                    password: percent_decode_compact(pass.unwrap_or(""))?,
                })
            }
            None => None,
        };
        let host_clean = a.host;
        let bracketed = a.host_bracketed;
        if host_clean.is_empty()
            || (!bracketed && host_clean.starts_with(':'))
            || host_clean.len() > 255
        {
            return Err(ProxyParseError::MissingHost);
        }
        let port = match a.port {
            Some(p) => {
                let v: u32 = p.parse().map_err(|_| ProxyParseError::BadPort)?;
                if v == 0 || v > u16::MAX as u32 {
                    return Err(ProxyParseError::BadPort);
                }
                v as u16
            }
            None => match scheme {
                ProxyScheme::Http => 80,
                ProxyScheme::Https => 443,
                ProxyScheme::Socks5 | ProxyScheme::Socks5h | ProxyScheme::Socks4 => {
                    return Err(ProxyParseError::MissingPort)
                }
            },
        };
        let target = match host_clean.parse::<core::net::IpAddr>() {
            Ok(ip) => TargetAddr::Ip(ip),
            Err(_) => TargetAddr::Domain(CompactString::from(host_clean)),
        };
        Ok(Self {
            scheme,
            target,
            port,
            auth,
            utc_offset: query_int(s, "utc").and_then(|v| i32::try_from(v).ok()),
            asn: query_int(s, "asn").and_then(|v| u32::try_from(v).ok()),
        })
    }

    #[inline]
    pub fn host(&self) -> CompactString {
        self.target.host_string()
    }

    pub fn to_url_string(&self) -> CompactString {
        let auth_len = self
            .auth
            .as_ref()
            .map(|a| a.username.len() * 3 + a.password.len() * 3 + 6)
            .unwrap_or(0);
        let host_len = match &self.target {
            TargetAddr::Ip(_) => 45,
            TargetAddr::Domain(d) => d.len(),
        };
        let mut out = CompactString::with_capacity(
            self.scheme.as_str().len() + host_len + auth_len + 8 + usize::from(self.target.is_ipv6()) * 2,
        );
        out.push_str(self.scheme.as_str());
        out.push_str("://");
        if let Some(a) = &self.auth {
            out.push_str(percent_encode_compact(a.username.as_str()).as_str());
            out.push(':');
            out.push_str(percent_encode_compact(a.password.as_str()).as_str());
            out.push('@');
        }
        self.write_authority(&mut out);
        out
    }

    fn write_authority(&self, out: &mut CompactString) {
        let v6 = self.target.is_ipv6();
        if v6 {
            out.push('[');
        }
        match &self.target {
            TargetAddr::Domain(d) => out.push_str(d.as_str()),
            TargetAddr::Ip(ip) => out.push_str(&ip.to_compact_string()),
        }
        if v6 {
            out.push(']');
        }
        out.push(':');
        core_utils::push_int_into(out, i64::from(self.port));
    }

    #[inline]
    pub const fn wreq_compatible(&self) -> bool {
        self.scheme.wreq_supported()
    }

    pub fn identity_key(&self) -> CompactString {
        let mut out = self.redacted_string();
        if let Some(a) = &self.auth {
            out.push('@');
            out.push_str(a.username.as_str());
        }
        out.push('#');
        if let Some(asn) = self.asn {
            core_utils::push_int_into(&mut out, i64::from(asn));
        }
        out.push('|');
        if let Some(utc) = self.utc_offset {
            core_utils::push_int_into(&mut out, i64::from(utc));
        }
        out
    }

    pub fn redacted_string(&self) -> CompactString {
        let host_len = match &self.target {
            TargetAddr::Ip(_) => 45,
            TargetAddr::Domain(d) => d.len(),
        };
        let mut out = CompactString::with_capacity(
            self.scheme.as_str().len() + host_len + 8 + usize::from(self.target.is_ipv6()) * 2,
        );
        out.push_str(self.scheme.as_str());
        out.push_str("://");
        self.write_authority(&mut out);
        out
    }
}

fn percent_decode_compact(s: &str) -> Result<CompactString, ProxyParseError> {
    CompactString::from_utf8(percent_decode(s)).map_err(|_| ProxyParseError::BadUserInfo)
}

fn query_int(s: &str, key: &str) -> Option<i64> {
    let q = s.find('?')?;
    let rest = &s[q + 1..];
    let mut scan = 0usize;
    loop {
        let rel = rest[scan..].find(key)? + scan;
        let boundary = rel == 0 || matches!(rest.as_bytes()[rel - 1], b'?' | b'&');
        if boundary && rest.as_bytes().get(rel + key.len()) == Some(&b'=') {
            let tail = &rest[rel + key.len() + 1..];
            let end = tail.find('&').unwrap_or(tail.len());
            return tail[..end].parse::<i64>().ok();
        }
        scan = rel + 1;
    }
}
