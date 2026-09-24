//! Version constraints such as `>=7.4 <8`, `^28`, `7.*` or `7.4`.
//!
//! Strict semantic versioning would reject what Windows tools actually report:
//! Windows itself is `10.0.26200.9457`, Git for Windows is `2.39.2.windows.1`.
//! So a version is its leading run of numeric components, compared component
//! by component with missing ones treated as zero, and anything after the
//! first non-numeric component is ignored.
//!
//! Grammar: alternatives separated by `||`; each alternative is one or more
//! comparators separated by spaces, all of which must hold. A comparator is an
//! optional operator (`=`, `>`, `>=`, `<`, `<=`, `^`, `~`) and a version, whose
//! trailing components may be `*` or `x`. A bare version is a prefix match:
//! `7.4` means any `7.4.x`.

use std::cmp::Ordering;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version(Vec<u64>);

impl Version {
    /// The leading numeric components of `text`, or `None` if it does not
    /// start with a number.
    pub fn parse_lenient(text: &str) -> Option<Self> {
        let text = text.trim().trim_start_matches(['v', 'V']);
        let mut parts = Vec::new();
        for component in text.split('.') {
            let digits: String = component.chars().take_while(char::is_ascii_digit).collect();
            if digits.is_empty() {
                break;
            }
            parts.push(digits.parse().ok()?);
            if digits.len() != component.len() {
                break; // "0-preview" ends the numeric run
            }
        }
        (!parts.is_empty()).then_some(Self(parts))
    }

    fn component(&self, i: usize) -> u64 {
        self.0.get(i).copied().unwrap_or(0)
    }

    fn cmp_to(&self, other: &[u64]) -> Ordering {
        let len = self.0.len().max(other.len());
        (0..len)
            .map(|i| self.component(i).cmp(other.get(i).unwrap_or(&0)))
            .find(|o| *o != Ordering::Equal)
            .unwrap_or(Ordering::Equal)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.0.iter().map(u64::to_string).collect();
        f.write_str(&parts.join("."))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Prefix,
    Gt,
    Ge,
    Lt,
    Le,
    Caret,
    Tilde,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Comparator {
    op: Op,
    /// The numeric components given, before any wildcard.
    parts: Vec<u64>,
}

impl Comparator {
    fn matches(&self, v: &Version) -> bool {
        let p = &self.parts;
        match self.op {
            Op::Prefix => p.iter().enumerate().all(|(i, n)| v.component(i) == *n),
            Op::Gt => v.cmp_to(p) == Ordering::Greater,
            Op::Ge => v.cmp_to(p) != Ordering::Less,
            Op::Lt => v.cmp_to(p) == Ordering::Less,
            Op::Le => v.cmp_to(p) != Ordering::Greater,
            // ^1.2.3 is >=1.2.3 <2; ^0.2.3 is >=0.2.3 <0.3, as in semver.
            Op::Caret => {
                let fixed = p.iter().position(|n| *n != 0).unwrap_or(p.len().saturating_sub(1));
                v.cmp_to(p) != Ordering::Less
                    && p[..=fixed].iter().enumerate().all(|(i, n)| v.component(i) == *n)
            }
            // ~1.2.3 is >=1.2.3 <1.3; ~1 is >=1 <2.
            Op::Tilde => {
                let fixed = if p.len() > 1 { 2 } else { 1 };
                v.cmp_to(p) != Ordering::Less
                    && p.iter().take(fixed).enumerate().all(|(i, n)| v.component(i) == *n)
            }
        }
    }
}

/// A parsed constraint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constraint {
    alternatives: Vec<Vec<Comparator>>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{text}` is not a version constraint: {reason}")]
pub struct ConstraintError {
    pub text: String,
    pub reason: String,
}

impl Constraint {
    pub fn parse(text: &str) -> Result<Self, ConstraintError> {
        let fail = |reason: &str| ConstraintError { text: text.to_owned(), reason: reason.to_owned() };
        let mut alternatives = Vec::new();
        for alt in text.split("||") {
            let mut comparators = Vec::new();
            for token in alt.split_whitespace() {
                comparators.push(parse_comparator(token).map_err(|r| fail(&r))?);
            }
            if comparators.is_empty() {
                return Err(fail("an alternative is empty"));
            }
            alternatives.push(comparators);
        }
        Ok(Self { alternatives })
    }

    pub fn matches(&self, version: &Version) -> bool {
        self.alternatives.iter().any(|alt| alt.iter().all(|c| c.matches(version)))
    }
}

fn parse_comparator(token: &str) -> Result<Comparator, String> {
    let (op, rest) = [
        (">=", Op::Ge),
        ("<=", Op::Le),
        (">", Op::Gt),
        ("<", Op::Lt),
        ("^", Op::Caret),
        ("~", Op::Tilde),
        ("=", Op::Prefix),
    ]
    .iter()
    .find_map(|(s, op)| token.strip_prefix(s).map(|r| (*op, r)))
    .unwrap_or((Op::Prefix, token));
    let mut parts = Vec::new();
    let mut wildcard = false;
    for component in rest.split('.') {
        match component {
            "*" | "x" | "X" => wildcard = true,
            _ if wildcard => return Err(format!("`{token}` has a number after a wildcard")),
            _ => parts.push(
                component
                    .parse::<u64>()
                    .map_err(|_| format!("`{component}` in `{token}` is not a number"))?,
            ),
        }
    }
    if parts.is_empty() {
        // A bare `*` matches anything; nothing else may be all wildcard.
        return if op == Op::Prefix && wildcard {
            Ok(Comparator { op, parts })
        } else {
            Err(format!("`{token}` names no version"))
        };
    }
    if wildcard && op != Op::Prefix {
        return Err(format!("`{token}` combines an operator with a wildcard"));
    }
    Ok(Comparator { op, parts })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(c: &str, v: &str) -> bool {
        Constraint::parse(c).unwrap().matches(&Version::parse_lenient(v).unwrap())
    }

    #[test]
    fn reads_versions_windows_tools_actually_report() {
        assert_eq!(Version::parse_lenient("10.0.26200.9457").unwrap().to_string(), "10.0.26200.9457");
        assert_eq!(Version::parse_lenient("2.39.2.windows.1").unwrap().to_string(), "2.39.2");
        assert_eq!(Version::parse_lenient("7.5.0-preview.3").unwrap().to_string(), "7.5.0");
        assert_eq!(Version::parse_lenient("v28.0.1").unwrap().to_string(), "28.0.1");
        assert!(Version::parse_lenient("unknown").is_none());
    }

    #[test]
    fn ranges_and_prefixes() {
        assert!(ok(">=7.4 <8", "7.6.6"));
        assert!(!ok(">=7.4 <8", "8.0.0"));
        assert!(!ok(">=7.4 <8", "7.3.9"));
        assert!(ok("7.4", "7.4.6"));
        assert!(!ok("7.4", "7.5.0"));
        assert!(ok("7.*", "7.6.6"));
        assert!(ok("*", "1.0"));
        assert!(ok("<5 || >=7", "7.1"));
        assert!(!ok("<5 || >=7", "6.0"));
        assert!(ok(">=10.0.22000", "10.0.26200.9457"), "Windows 11 check");
    }

    #[test]
    fn caret_and_tilde_follow_semver() {
        assert!(ok("^28", "28.3.1"));
        assert!(!ok("^28", "29.0"));
        assert!(ok("^0.2.3", "0.2.9"));
        assert!(!ok("^0.2.3", "0.3.0"));
        assert!(ok("~1.2.3", "1.2.9"));
        assert!(!ok("~1.2.3", "1.3.0"));
        assert!(ok("~1", "1.9"));
    }

    #[test]
    fn nonsense_is_rejected_up_front() {
        for bad in ["", ">=", "7.*.1", ">=7.*", "seven", "1 ||", ">>7"] {
            assert!(Constraint::parse(bad).is_err(), "{bad:?}");
        }
    }
}
