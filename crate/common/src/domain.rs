use std::fmt;
use std::str::FromStr;

use chrono::{Datelike, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cadence {
    Weekly,
    Monthly,
    Yearly,
}

impl Cadence {
    pub fn as_str(self) -> &'static str {
        match self {
            Cadence::Weekly => "weekly",
            Cadence::Monthly => "monthly",
            Cadence::Yearly => "yearly",
        }
    }
}

impl fmt::Display for Cadence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Cadence {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "weekly" => Ok(Cadence::Weekly),
            "monthly" => Ok(Cadence::Monthly),
            "yearly" => Ok(Cadence::Yearly),
            other => Err(Error::InvalidCadence(other.to_string())),
        }
    }
}

use crate::error::{Error, Result};

macro_rules! id_newtype {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id_newtype!(UserId);
id_newtype!(MembershipId);
id_newtype!(ContributionId);
id_newtype!(InviteId);

/// Tenant identity = SurrealDB namespace name. The slug IS the tenant —
/// there is no `tenant` row anywhere. Validation must match the regex used
/// by the schema (`^[a-z][a-z0-9_]{2,40}$`) so what passes here is also
/// safe to interpolate as a namespace name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TenantSlug(String);

impl TenantSlug {
    pub fn new(s: impl Into<String>) -> Result<Self> {
        let s: String = s.into();
        if !is_valid_slug(&s) {
            return Err(Error::InvalidTenantSlug(s));
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for TenantSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for TenantSlug {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Self::new(s)
    }
}

fn is_valid_slug(s: &str) -> bool {
    let len = s.len();
    if !(3..=41).contains(&len) {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_lowercase() {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// An accounting period. Shape depends on the venture's `Cadence`:
///   weekly  → "YYYY-Www" (ISO 8601 week-year + week)
///   monthly → "YYYY-MM"
///   yearly  → "YYYY"
///
/// Tenant-scoped: every tenant has its own timezone (stored in
/// `settings:current` in its namespace), and `Period::current_in(tz, cadence)`
/// returns the period that "right now" falls into for that tenant.
///
/// Ordering across cadences is undefined and intentionally not derived;
/// the lock check compares within a row's own cadence, where lexicographic
/// `String` comparison is correct because all formats are zero-padded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Period {
    Weekly { year: i32, week: u8 },
    Monthly { year: i32, month: u8 },
    Yearly { year: i32 },
}

impl Period {
    pub fn weekly(year: i32, week: u8) -> Result<Self> {
        if !(1..=53).contains(&week) {
            return Err(Error::InvalidPeriod(format!("{year}-W{week:02}")));
        }
        Ok(Self::Weekly { year, week })
    }

    pub fn monthly(year: i32, month: u8) -> Result<Self> {
        if !(1..=12).contains(&month) {
            return Err(Error::InvalidPeriod(format!("{year}-{month:02}")));
        }
        Ok(Self::Monthly { year, month })
    }

    pub fn yearly(year: i32) -> Self {
        Self::Yearly { year }
    }

    pub fn current_in(tz: Tz, cadence: Cadence) -> Self {
        let local = Utc::now().with_timezone(&tz);
        match cadence {
            Cadence::Weekly => {
                let iso = local.iso_week();
                Self::Weekly {
                    year: iso.year(),
                    week: iso.week() as u8,
                }
            }
            Cadence::Monthly => Self::Monthly {
                year: local.year(),
                month: local.month() as u8,
            },
            Cadence::Yearly => Self::Yearly { year: local.year() },
        }
    }

    pub fn cadence(&self) -> Cadence {
        match self {
            Period::Weekly { .. } => Cadence::Weekly,
            Period::Monthly { .. } => Cadence::Monthly,
            Period::Yearly { .. } => Cadence::Yearly,
        }
    }
}

impl fmt::Display for Period {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Period::Weekly { year, week } => write!(f, "{year:04}-W{week:02}"),
            Period::Monthly { year, month } => write!(f, "{year:04}-{month:02}"),
            Period::Yearly { year } => write!(f, "{year:04}"),
        }
    }
}

impl FromStr for Period {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        // Yearly: "YYYY"
        if s.len() == 4 {
            let year: i32 = s.parse().map_err(|_| Error::InvalidPeriod(s.into()))?;
            return Ok(Self::Yearly { year });
        }
        // Otherwise expect "YYYY-..." with the second segment distinguishing.
        let (y, rest) = s
            .split_once('-')
            .ok_or_else(|| Error::InvalidPeriod(s.into()))?;
        let year: i32 = y.parse().map_err(|_| Error::InvalidPeriod(s.into()))?;
        if let Some(week_str) = rest.strip_prefix('W') {
            let week: u8 = week_str
                .parse()
                .map_err(|_| Error::InvalidPeriod(s.into()))?;
            Self::weekly(year, week)
        } else {
            let month: u8 = rest.parse().map_err(|_| Error::InvalidPeriod(s.into()))?;
            Self::monthly(year, month)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Treasurer,
    Member,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Treasurer => "treasurer",
            Role::Member => "member",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContributionStatus {
    Submitted,
    Verified,
    Rejected,
}

/// IANA timezone string parsed into `Tz`.
pub fn parse_tz(name: &str) -> Result<Tz> {
    name.parse::<Tz>()
        .map_err(|_| Error::InvalidTimezone(name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn period_monthly_roundtrip() {
        let p = Period::monthly(2026, 4).unwrap();
        assert_eq!(p.to_string(), "2026-04");
        assert_eq!("2026-04".parse::<Period>().unwrap(), p);
        assert_eq!(p.cadence(), Cadence::Monthly);
    }

    #[test]
    fn period_weekly_roundtrip() {
        let p = Period::weekly(2026, 18).unwrap();
        assert_eq!(p.to_string(), "2026-W18");
        assert_eq!("2026-W18".parse::<Period>().unwrap(), p);
        assert_eq!(p.cadence(), Cadence::Weekly);
    }

    #[test]
    fn period_yearly_roundtrip() {
        let p = Period::yearly(2026);
        assert_eq!(p.to_string(), "2026");
        assert_eq!("2026".parse::<Period>().unwrap(), p);
        assert_eq!(p.cadence(), Cadence::Yearly);
    }

    #[test]
    fn period_invalid_month() {
        assert!(Period::monthly(2026, 13).is_err());
        assert!(Period::monthly(2026, 0).is_err());
    }

    #[test]
    fn period_invalid_week() {
        assert!(Period::weekly(2026, 0).is_err());
        assert!(Period::weekly(2026, 54).is_err());
    }

    #[test]
    fn period_lex_ordering_within_cadence() {
        // Lock check relies on lexicographic string comparison being correct
        // within a single cadence — verify that's true for each.
        assert!(
            Period::monthly(2026, 4).unwrap().to_string()
                < Period::monthly(2026, 5).unwrap().to_string()
        );
        assert!(
            Period::monthly(2026, 12).unwrap().to_string()
                < Period::monthly(2027, 1).unwrap().to_string()
        );
        assert!(
            Period::weekly(2026, 9).unwrap().to_string()
                < Period::weekly(2026, 10).unwrap().to_string()
        );
        assert!(Period::yearly(2026).to_string() < Period::yearly(2027).to_string());
    }

    #[test]
    fn cadence_roundtrip() {
        for c in [Cadence::Weekly, Cadence::Monthly, Cadence::Yearly] {
            assert_eq!(c.to_string().parse::<Cadence>().unwrap(), c);
        }
        assert!("biweekly".parse::<Cadence>().is_err());
    }

    #[test]
    fn parse_tz_known() {
        assert!(parse_tz("Asia/Kuala_Lumpur").is_ok());
        assert!(parse_tz("UTC").is_ok());
        assert!(parse_tz("Not/AReal_Zone").is_err());
    }

    #[test]
    fn tenant_slug_valid() {
        assert!(TenantSlug::new("acme_capital").is_ok());
        assert!(TenantSlug::new("a01_test").is_ok());
        assert!(TenantSlug::new("abc").is_ok());
    }

    #[test]
    fn tenant_slug_invalid() {
        // too short
        assert!(TenantSlug::new("ab").is_err());
        // starts with digit
        assert!(TenantSlug::new("1capital").is_err());
        // uppercase
        assert!(TenantSlug::new("Acme").is_err());
        // hyphen not allowed
        assert!(TenantSlug::new("acme-capital").is_err());
        // empty
        assert!(TenantSlug::new("").is_err());
    }
}
