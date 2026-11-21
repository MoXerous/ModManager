//! F-097: judging a declared Minecraft version declaration against one target.
//!
//! The old app compared the target with each declared value as a raw string, so
//! a provider that declared `>=1.20` was read as "not 1.20.1" and the file was
//! called incompatible. A range is a claim about a *set*, and membership is the
//! only question that can be answered from it.
//!
//! Three outcomes, because "not proven" is not "proven absent":
//!
//! - [`RangeVerdict::Includes`] — the declaration admits the target.
//! - [`RangeVerdict::Excludes`] — every reading of the declaration excludes it.
//! - [`RangeVerdict::Unparseable`] — this build cannot read the declaration, so
//!   nothing is concluded. It must never become an incompatibility.
//!
//! Dependency-free by design: the forms handled are the ones the catalogues and
//! the jars actually emit (Maven/Forge ranges, Cargo-style `^`/`~`, npm-style
//! `x`, and a plain exact version). Anything else is `Unparseable` rather than
//! a guess.

/// What a declaration says about one target version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeVerdict {
    Includes,
    Excludes,
    Unparseable,
}

/// Judges one declaration against one target.
///
/// A declaration equal to the target is `Includes` before any parsing: the
/// catalogues also publish snapshot names (`24w14a`) that have no numeric
/// order, and an exact spelling is still an exact answer.
pub fn judge(declaration: &str, target: &str) -> RangeVerdict {
    let declaration = declaration.trim();
    let target = target.trim();
    if declaration.is_empty() || target.is_empty() {
        return RangeVerdict::Unparseable;
    }
    if declaration.eq_ignore_ascii_case(target) {
        return RangeVerdict::Includes;
    }
    // A release with a non-numeric segment (`24w14a`, `1.20-pre1`) has no
    // numeric order here; a range that names one cannot be judged.
    let Some(target) = Version::parse(target) else {
        return RangeVerdict::Unparseable;
    };
    let Some(range) = Range::parse(declaration) else {
        return RangeVerdict::Unparseable;
    };
    if range.contains(&target) {
        RangeVerdict::Includes
    } else {
        RangeVerdict::Excludes
    }
}

/// A dotted numeric release. Segments are padded with zeros on comparison, so
/// `1.20` and `1.20.0` are one release.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Version(Vec<u64>);

impl Version {
    fn parse(raw: &str) -> Option<Self> {
        let mut parts = Vec::new();
        for part in raw.split('.') {
            if part.is_empty() {
                return None;
            }
            parts.push(part.parse::<u64>().ok()?);
        }
        if parts.is_empty() {
            None
        } else {
            Some(Version(parts))
        }
    }
}

/// Compares two releases segment by segment, a missing segment counting as 0.
fn compare(left: &[u64], right: &[u64]) -> std::cmp::Ordering {
    for index in 0..left.len().max(right.len()) {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        match a.cmp(&b) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

/// One bound of an interval: the version and whether the bound itself counts.
struct Bound {
    version: Version,
    inclusive: bool,
}

/// The declaration forms this build reads.
enum Range {
    Exact(Version),
    GreaterOrEqual(Version),
    Greater(Version),
    LessOrEqual(Version),
    Less(Version),
    /// `[a,b)` and friends, and the `^`/`~` shorthands once resolved.
    Interval {
        lower: Option<Bound>,
        upper: Option<Bound>,
    },
    /// `1.20.x`: every release whose first N segments are the given ones.
    Wildcard(Vec<u64>),
}

impl Range {
    fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        if raw.starts_with('[') || raw.starts_with('(') {
            return Self::parse_interval(raw);
        }
        if let Some(rest) = raw.strip_prefix(">=") {
            return Version::parse(rest).map(Range::GreaterOrEqual);
        }
        if let Some(rest) = raw.strip_prefix("<=") {
            return Version::parse(rest).map(Range::LessOrEqual);
        }
        if let Some(rest) = raw.strip_prefix('>') {
            return Version::parse(rest).map(Range::Greater);
        }
        if let Some(rest) = raw.strip_prefix('<') {
            return Version::parse(rest).map(Range::Less);
        }
        if let Some(rest) = raw.strip_prefix('^') {
            return Version::parse(rest).map(caret);
        }
        if let Some(rest) = raw.strip_prefix('~') {
            return Version::parse(rest).map(tilde);
        }
        if let Some(rest) = raw.strip_prefix('=') {
            return Version::parse(rest).map(Range::Exact);
        }
        if raw.contains(['x', 'X', '*']) {
            return Self::parse_wildcard(raw);
        }
        Version::parse(raw).map(Range::Exact)
    }

    /// `[1.20.1,1.21.2)`, `(1.20.1,1.21.2]`, `[1.20.1,)`, `(,1.21]`, `[1.20.1]`.
    /// An empty bound is unbounded; a bracket pair is inclusive, a parenthesis
    /// is not. Exactly one comma is the only shape read.
    fn parse_interval(raw: &str) -> Option<Self> {
        let upper_inclusive = raw.ends_with(']');
        if !upper_inclusive && !raw.ends_with(')') {
            return None;
        }
        let lower_inclusive = raw.starts_with('[');
        let inner = &raw[1..raw.len() - 1];
        match inner.split(',').collect::<Vec<_>>().as_slice() {
            [single] => {
                if lower_inclusive && upper_inclusive {
                    Version::parse(single.trim()).map(Range::Exact)
                } else {
                    None
                }
            }
            [lower, upper] => {
                let lower = lower.trim();
                let upper = upper.trim();
                let lower = if lower.is_empty() {
                    None
                } else {
                    Some(Bound {
                        version: Version::parse(lower)?,
                        inclusive: lower_inclusive,
                    })
                };
                let upper = if upper.is_empty() {
                    None
                } else {
                    Some(Bound {
                        version: Version::parse(upper)?,
                        inclusive: upper_inclusive,
                    })
                };
                if lower.is_none() && upper.is_none() {
                    return None;
                }
                Some(Range::Interval { lower, upper })
            }
            _ => None,
        }
    }

    /// `1.20.x`, `1.x`: a numeric prefix and a trailing wildcard. A numeric
    /// segment after the wildcard (`1.x.3`) is not a shape we read, and a bare
    /// `x`/`*` names the whole catalogue — including names with no numeric
    /// order — so neither is guessed at.
    fn parse_wildcard(raw: &str) -> Option<Self> {
        let mut prefix = Vec::new();
        let mut saw_wildcard = false;
        for part in raw.split('.') {
            if part.eq_ignore_ascii_case("x") || part == "*" {
                saw_wildcard = true;
                continue;
            }
            if saw_wildcard {
                return None;
            }
            prefix.push(part.parse::<u64>().ok()?);
        }
        if !saw_wildcard || prefix.is_empty() {
            return None;
        }
        Some(Range::Wildcard(prefix))
    }

    fn contains(&self, target: &Version) -> bool {
        match self {
            Range::Exact(version) => compare(&version.0, &target.0).is_eq(),
            Range::GreaterOrEqual(version) => !compare(&target.0, &version.0).is_lt(),
            Range::Greater(version) => compare(&target.0, &version.0).is_gt(),
            Range::LessOrEqual(version) => !compare(&target.0, &version.0).is_gt(),
            Range::Less(version) => compare(&target.0, &version.0).is_lt(),
            Range::Interval { lower, upper } => {
                let above = match lower {
                    None => true,
                    Some(bound) if bound.inclusive => !compare(&target.0, &bound.version.0).is_lt(),
                    Some(bound) => compare(&target.0, &bound.version.0).is_gt(),
                };
                let below = match upper {
                    None => true,
                    Some(bound) if bound.inclusive => !compare(&target.0, &bound.version.0).is_gt(),
                    Some(bound) => compare(&target.0, &bound.version.0).is_lt(),
                };
                above && below
            }
            Range::Wildcard(prefix) => (0..prefix.len())
                .all(|index| target.0.get(index).copied().unwrap_or(0) == prefix[index]),
        }
    }
}

/// `^1.20.1` ⇒ `>=1.20.1, <2.0.0`: the first non-zero segment may not change.
/// `^0.2.3` ⇒ `<0.3.0`, `^0.0.3` ⇒ `<0.0.4` — Cargo's rule, which is what the
/// catalogues mean when they emit it.
fn caret(lower: Version) -> Range {
    let parts = &lower.0;
    let upper = match parts.iter().position(|part| *part != 0) {
        // All zeros: only the patch position may move.
        None => vec![1],
        Some(index) => {
            let mut upper = parts[..=index].to_vec();
            if let Some(last) = upper.last_mut() {
                *last += 1;
            }
            upper
        }
    };
    Range::Interval {
        lower: Some(Bound {
            version: lower,
            inclusive: true,
        }),
        upper: Some(Bound {
            version: Version(upper),
            inclusive: false,
        }),
    }
}

/// `~1.20.1` ⇒ `>=1.20.1, <1.21.0`; `~1.20` ⇒ `>=1.20, <1.21`; `~1` ⇒ `<2.0.0`.
/// Patch-level changes only, the npm reading the catalogues use.
fn tilde(lower: Version) -> Range {
    let upper = match lower.0.as_slice() {
        [major] => vec![major + 1],
        [major, minor, ..] => vec![*major, minor + 1],
        // `Version::parse` never yields an empty release; the fallback keeps
        // this total without an unreachable panic.
        [] => vec![1],
    };
    Range::Interval {
        lower: Some(Bound {
            version: lower,
            inclusive: true,
        }),
        upper: Some(Bound {
            version: Version(upper),
            inclusive: false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F-097: every form the catalogues and jars emit, judged against a target
    /// it admits. A range that includes the target is compatible — the whole
    /// point of parsing it instead of comparing strings.
    #[test]
    fn spec_f097_range_forms_include_the_target() {
        let cases = [
            ("1.20.1", "1.20.1"),
            ("1.20", "1.20.0"),
            (">=1.20.1", "1.21.1"),
            (">=1.20.1", "1.20.1"),
            ("<=1.21.1", "1.20.1"),
            ("<1.21", "1.20.1"),
            (">1.20", "1.20.1"),
            ("=1.20.1", "1.20.1"),
            ("^1.20.1", "1.21.1"),
            ("~1.20.1", "1.20.9"),
            ("1.20.x", "1.20.4"),
            ("1.x", "1.21.1"),
            ("[1.20.1,1.21.2)", "1.20.1"),
            ("[1.20.1,1.21.2)", "1.21.1"),
            ("(1.20.1,1.21.2]", "1.21.2"),
            ("[1.20.1]", "1.20.1"),
            ("[1.20.1,)", "1.30"),
            ("(,1.21]", "1.20.9"),
            ("24w14a", "24w14a"),
        ];
        for (declaration, target) in cases {
            assert_eq!(
                judge(declaration, target),
                RangeVerdict::Includes,
                "{declaration} must include {target}"
            );
        }
    }

    /// F-097: the other direction. A range that excludes the target is
    /// incompatible — including the case the old app got wrong, `>=1.21`
    /// against a `1.20.1` instance.
    #[test]
    fn spec_f097_range_forms_exclude_the_target() {
        let cases = [
            ("1.20.1", "1.20.2"),
            (">=1.21", "1.20.1"),
            ("<=1.20", "1.20.1"),
            ("<1.20.1", "1.20.1"),
            (">1.20.1", "1.20.1"),
            ("=1.20.1", "1.20.2"),
            ("^1.21", "1.20.1"),
            ("~1.20.1", "1.21.0"),
            ("1.20.x", "1.21.1"),
            ("[1.20.1,1.21.2)", "1.21.2"),
            ("(1.20.1,1.21.2]", "1.20.1"),
            ("(1.20.1,1.21.2]", "1.21.3"),
        ];
        for (declaration, target) in cases {
            assert_eq!(
                judge(declaration, target),
                RangeVerdict::Excludes,
                "{declaration} must exclude {target}"
            );
        }
    }

    /// F-097: the rule that keeps the incompatible class honest. A declaration
    /// this build cannot read settles nothing: it is `Unparseable`, never
    /// `Excludes`, so the caller answers `Unknown` instead of inventing a
    /// contradiction the metadata never asserted.
    #[test]
    fn spec_f097_an_unparseable_declaration_is_never_excluded() {
        let declarations = [
            "",
            "   ",
            "1.20 || 1.21",
            "1.20 - 1.21",
            ">=abc",
            "latest",
            "1.20.x-pre",
            "*",
        ];
        for declaration in declarations {
            assert_eq!(
                judge(declaration, "1.20.1"),
                RangeVerdict::Unparseable,
                "{declaration:?} must not become a verdict"
            );
        }
        assert_eq!(
            judge("24w14a", "1.20.1"),
            RangeVerdict::Unparseable,
            "a snapshot name is not ordered against a release"
        );
    }
}
