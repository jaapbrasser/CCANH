//! HTTP authentication challenge parsing and Digest responses (RFC 7616 /
//! RFC 2617), as used by AXIS devices (VAPIX "Authentication").

use md5::{Digest as _, Md5};
use sha2::Sha256;

/// A parsed `WWW-Authenticate` challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Challenge {
    Basic,
    Digest(DigestChallenge),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestChallenge {
    pub realm: String,
    pub nonce: String,
    pub opaque: Option<String>,
    pub algorithm: Algorithm,
    /// Whether the server offered `qop="auth"`.
    pub qop_auth: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    Md5,
    Md5Sess,
    Sha256,
    Sha256Sess,
}

impl Algorithm {
    fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "MD5" => Some(Algorithm::Md5),
            "MD5-SESS" => Some(Algorithm::Md5Sess),
            "SHA-256" => Some(Algorithm::Sha256),
            "SHA-256-SESS" => Some(Algorithm::Sha256Sess),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Algorithm::Md5 => "MD5",
            Algorithm::Md5Sess => "MD5-sess",
            Algorithm::Sha256 => "SHA-256",
            Algorithm::Sha256Sess => "SHA-256-sess",
        }
    }

    fn hash(self, data: &str) -> String {
        let bytes: Vec<u8> = match self {
            Algorithm::Md5 | Algorithm::Md5Sess => Md5::digest(data.as_bytes()).to_vec(),
            Algorithm::Sha256 | Algorithm::Sha256Sess => Sha256::digest(data.as_bytes()).to_vec(),
        };
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn rank(self) -> u8 {
        match self {
            Algorithm::Sha256 | Algorithm::Sha256Sess => 2,
            Algorithm::Md5 | Algorithm::Md5Sess => 1,
        }
    }
}

/// Split `key=value, key="quoted, value"` parameters.
fn params(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s.trim();
    while !rest.is_empty() {
        let Some(eq) = rest.find('=') else { break };
        let key = rest[..eq]
            .trim()
            .trim_start_matches(',')
            .trim()
            .to_ascii_lowercase();
        rest = rest[eq + 1..].trim_start();
        let value;
        if let Some(stripped) = rest.strip_prefix('"') {
            let mut v = String::new();
            let mut chars = stripped.char_indices();
            let mut end = stripped.len();
            while let Some((i, c)) = chars.next() {
                match c {
                    '\\' => {
                        if let Some((_, n)) = chars.next() {
                            v.push(n);
                        }
                    }
                    '"' => {
                        end = i + 1;
                        break;
                    }
                    _ => v.push(c),
                }
            }
            value = v;
            rest = stripped[end.min(stripped.len())..].trim_start();
        } else {
            let end = rest.find(',').unwrap_or(rest.len());
            value = rest[..end].trim().to_owned();
            rest = &rest[end..];
        }
        rest = rest.trim_start_matches(',').trim_start();
        out.push((key, value));
    }
    out
}

/// Parse all challenges from the `WWW-Authenticate` header values. A single
/// header may contain several challenges.
pub fn parse_challenges<'a>(headers: impl IntoIterator<Item = &'a str>) -> Vec<Challenge> {
    let mut out = Vec::new();
    for header in headers {
        // Split before each scheme token at the start or after a comma.
        let mut starts: Vec<usize> = Vec::new();
        let lower = header.to_ascii_lowercase();
        for scheme in ["basic", "digest"] {
            let mut from = 0;
            while let Some(pos) = lower[from..].find(scheme) {
                let at = from + pos;
                let before_ok = at == 0 || lower[..at].trim_end().ends_with(',');
                let after = lower[at + scheme.len()..].chars().next();
                if before_ok && matches!(after, None | Some(' ')) {
                    starts.push(at);
                }
                from = at + scheme.len();
            }
        }
        starts.sort_unstable();
        for (i, &start) in starts.iter().enumerate() {
            let end = starts.get(i + 1).copied().unwrap_or(header.len());
            let part = header[start..end].trim().trim_end_matches(',');
            if part.len() >= 5 && part[..5].eq_ignore_ascii_case("basic") {
                out.push(Challenge::Basic);
            } else if part.len() >= 6 && part[..6].eq_ignore_ascii_case("digest") {
                let p = params(&part[6..]);
                let get = |k: &str| p.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
                let algorithm = match get("algorithm") {
                    None => Some(Algorithm::Md5),
                    Some(a) => Algorithm::parse(&a),
                };
                if let (Some(realm), Some(nonce), Some(algorithm)) =
                    (get("realm"), get("nonce"), algorithm)
                {
                    out.push(Challenge::Digest(DigestChallenge {
                        realm,
                        nonce,
                        opaque: get("opaque"),
                        algorithm,
                        qop_auth: get("qop").is_some_and(|q| {
                            q.split(',').any(|v| v.trim().eq_ignore_ascii_case("auth"))
                        }),
                    }));
                }
            }
        }
    }
    out
}

/// The strongest Digest challenge offered, if any.
pub fn best_digest(challenges: &[Challenge]) -> Option<&DigestChallenge> {
    challenges
        .iter()
        .filter_map(|c| match c {
            Challenge::Digest(d) => Some(d),
            Challenge::Basic => None,
        })
        .max_by_key(|d| d.algorithm.rank())
}

/// Build an `Authorization: Digest …` header value.
pub fn digest_authorization(
    ch: &DigestChallenge,
    username: &str,
    password: &str,
    method: &str,
    uri: &str,
    nc: u32,
    cnonce: &str,
) -> String {
    let alg = ch.algorithm;
    let mut ha1 = alg.hash(&format!("{username}:{}:{password}", ch.realm));
    if matches!(alg, Algorithm::Md5Sess | Algorithm::Sha256Sess) {
        ha1 = alg.hash(&format!("{ha1}:{}:{cnonce}", ch.nonce));
    }
    let ha2 = alg.hash(&format!("{method}:{uri}"));
    let nc = format!("{nc:08x}");
    let response = if ch.qop_auth {
        alg.hash(&format!("{ha1}:{}:{nc}:{cnonce}:auth:{ha2}", ch.nonce))
    } else {
        alg.hash(&format!("{ha1}:{}:{ha2}", ch.nonce))
    };
    let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let mut h = format!(
        "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", algorithm={}, response=\"{}\"",
        quote(username),
        quote(&ch.realm),
        quote(&ch.nonce),
        quote(uri),
        alg.name(),
        response
    );
    if ch.qop_auth {
        h.push_str(&format!(", qop=auth, nc={nc}, cnonce=\"{cnonce}\""));
    }
    if let Some(opaque) = &ch.opaque {
        h.push_str(&format!(", opaque=\"{}\"", quote(opaque)));
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc2617_example() {
        // RFC 2617 §3.5 example.
        let ch = DigestChallenge {
            realm: "testrealm@host.com".into(),
            nonce: "dcd98b7102dd2f0e8b11d0f600bfb0c093".into(),
            opaque: Some("5ccc069c403ebaf9f0171e9517f40e41".into()),
            algorithm: Algorithm::Md5,
            qop_auth: true,
        };
        let h = digest_authorization(
            &ch,
            "Mufasa",
            "Circle Of Life",
            "GET",
            "/dir/index.html",
            1,
            "0a4f113b",
        );
        assert!(
            h.contains("response=\"6629fae49393a05397450978507c4ef1\""),
            "{h}"
        );
        assert!(h.contains("nc=00000001"));
    }

    #[test]
    fn rfc7616_sha256_example() {
        // RFC 7616 §3.9.1 example (SHA-256).
        let ch = DigestChallenge {
            realm: "http-auth@example.org".into(),
            nonce: "7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v".into(),
            opaque: Some("FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS".into()),
            algorithm: Algorithm::Sha256,
            qop_auth: true,
        };
        let h = digest_authorization(
            &ch,
            "Mufasa",
            "Circle of Life",
            "GET",
            "/dir/index.html",
            1,
            "f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ",
        );
        assert!(
            h.contains(
                "response=\"753927fa0e85d155564e2e272a28d1802ca10daf4496794697cf8db5856cb6c1\""
            ),
            "{h}"
        );
    }

    #[test]
    fn parses_multiple_challenges_and_prefers_sha256() {
        let headers = [
            r#"Digest realm="AXIS_ACCC8E000000", nonce="abc", algorithm=MD5, qop="auth""#,
            r#"Digest realm="AXIS_ACCC8E000000", nonce="def", algorithm=SHA-256, qop="auth", Basic realm="AXIS_ACCC8E000000""#,
        ];
        let ch = parse_challenges(headers);
        assert_eq!(ch.len(), 3);
        assert!(ch.contains(&Challenge::Basic));
        let best = best_digest(&ch).unwrap();
        assert_eq!(best.algorithm, Algorithm::Sha256);
        assert_eq!(best.nonce, "def");
    }

    #[test]
    fn handles_quoted_commas_and_missing_algorithm() {
        let ch = parse_challenges([r#"Digest realm="a, b", nonce="n1", qop="auth,auth-int""#]);
        let Challenge::Digest(d) = &ch[0] else {
            panic!()
        };
        assert_eq!(d.realm, "a, b");
        assert_eq!(d.algorithm, Algorithm::Md5);
        assert!(d.qop_auth);
        assert_eq!(
            parse_challenges(["Basic realm=\"x\""]),
            vec![Challenge::Basic]
        );
        assert!(parse_challenges(["Bearer realm=\"x\""]).is_empty());
    }
}
