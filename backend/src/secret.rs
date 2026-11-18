//! Secret handling. Secret values are wrapped in [`SecretString`] (from the
//! `secrecy` crate), which redacts `Debug`, has no `Serialize`/`Display`, and
//! zeroes memory on drop. The value is only reachable via
//! [`ExposeSecret::expose_secret`], which keeps every use greppable.

pub use secrecy::{ExposeSecret, SecretString};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_is_redacted_and_deserialize_works() {
        #[derive(serde::Deserialize, Debug)]
        struct Body {
            password: SecretString,
        }
        let body: Body = serde_json::from_str(r#"{"password":"hunter2-p4ss"}"#).unwrap();
        assert_eq!(body.password.expose_secret(), "hunter2-p4ss");
        assert!(!format!("{body:?}").contains("hunter2"));
    }
}
