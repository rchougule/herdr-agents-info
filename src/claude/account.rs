//! `~/.claude.json` reader for the optional `login` / `org` tokens.
//! Built but gated behind `[tokens] login` (default off).

use std::path::PathBuf;

/// The two account values, either of which may be absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Account {
    pub email: Option<String>,
    pub org: Option<String>,
}

impl Account {
    /// Parse from a `~/.claude.json` string.
    pub fn from_json_str(s: &str) -> Option<Account> {
        let v: serde_json::Value = serde_json::from_str(s).ok()?;
        let oauth = v.get("oauthAccount")?;
        let get = |k: &str| {
            oauth
                .get(k)
                .and_then(|x| x.as_str())
                .map(str::to_string)
                .filter(|s| !s.is_empty())
        };
        Some(Account {
            email: get("emailAddress"),
            org: get("organizationName"),
        })
    }

    /// Load from `~/.claude.json`. Missing file → empty account (never errors).
    pub fn load() -> Account {
        let Some(home) = home_dir() else {
            return Account::default();
        };
        let path = home.join(".claude.json");
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| Account::from_json_str(&s))
            .unwrap_or_default()
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_email_and_org() {
        let j = r#"{"oauthAccount":{"emailAddress":"user@example.com",
            "organizationName":"Example Org","accountUuid":"x"}}"#;
        let a = Account::from_json_str(j).unwrap();
        assert_eq!(a.email.as_deref(), Some("user@example.com"));
        assert_eq!(a.org.as_deref(), Some("Example Org"));
    }

    #[test]
    fn missing_oauth_returns_none() {
        assert!(Account::from_json_str(r#"{"other":1}"#).is_none());
    }

    #[test]
    fn empty_strings_become_none() {
        let a = Account::from_json_str(r#"{"oauthAccount":{"emailAddress":""}}"#).unwrap();
        assert!(a.email.is_none());
        assert!(a.org.is_none());
    }
}
