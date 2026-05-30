//! models

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::ops::Add;

use crate::{Error, Result};

pub mod admission;
pub mod appointment;
pub mod appointment_series;
pub mod billing;
pub mod communication;
pub mod consent;
pub mod coverage;
pub mod encounter;
pub mod facility;
pub mod identifier;
pub mod patient;
pub mod practitioner;
pub mod rtt;
pub mod schedule;
pub mod waitlist;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Gender {
    Male,
    Female,
    Other,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NameUse {
    Usual,
    Official,
    Temp,
    Nickname,
    Anonymous,
    Old,
    Maiden,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AddressUse {
    Home,
    Work,
    Temp,
    Old,
    Billing,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Address {
    pub use_type: Option<AddressUse>,
    pub line1: Option<String>,
    pub line2: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ContactPointSystem {
    Phone,
    Fax,
    Email,
    Pager,
    Url,
    Sms,
    Other,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ContactPointUse {
    Home,
    Work,
    Temp,
    Old,
    Mobile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactPoint {
    pub system: ContactPointSystem,
    pub value: String,
    pub use_type: Option<ContactPointUse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Iso4217(pub String);

impl Iso4217 {
    pub fn new(code: &str) -> Result<Self> {
        if code.len() != 3 {
            return Err(Error::validation(format!(
                "ISO 4217 code must be 3 characters, got {}",
                code.len()
            )));
        }
        if !code.chars().all(|c| c.is_ascii_uppercase()) {
            return Err(Error::validation(format!(
                "ISO 4217 code must be uppercase ASCII letters, got {:?}",
                code
            )));
        }
        Ok(Self(code.to_string()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Money {
    pub amount: Decimal,
    pub currency: Iso4217,
}

impl Money {
    pub fn new(amount: Decimal, currency: Iso4217) -> Self {
        Self { amount, currency }
    }

    pub fn zero(currency: Iso4217) -> Self {
        Self {
            amount: Decimal::ZERO,
            currency,
        }
    }

    pub fn try_add(self, other: Money) -> Result<Money> {
        if self.currency != other.currency {
            return Err(Error::validation(format!(
                "Cannot add Money with different currencies: {} vs {}",
                self.currency.0, other.currency.0
            )));
        }
        Ok(Money {
            amount: self.amount + other.amount,
            currency: self.currency,
        })
    }
}

impl Add for Money {
    type Output = Money;

    fn add(self, rhs: Money) -> Money {
        assert_eq!(
            self.currency, rhs.currency,
            "Cannot add Money with different currencies"
        );
        Money {
            amount: self.amount + rhs.amount,
            currency: self.currency,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl TimeRange {
    pub fn is_valid(&self) -> bool {
        self.start < self.end
    }

    pub fn overlaps(&self, other: &TimeRange) -> bool {
        self.start < other.end && other.start < self.end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rust_decimal::Decimal;

    #[test]
    fn test_iso4217_accepts_usd() {
        let c = Iso4217::new("USD").expect("USD should be valid");
        assert_eq!(c.0, "USD");
    }

    #[test]
    fn test_iso4217_rejects_lowercase() {
        assert!(Iso4217::new("us").is_err());
        assert!(Iso4217::new("usd").is_err());
    }

    #[test]
    fn test_iso4217_rejects_wrong_length() {
        assert!(Iso4217::new("USDD").is_err());
        assert!(Iso4217::new("US").is_err());
        assert!(Iso4217::new("").is_err());
    }

    #[test]
    fn test_money_try_add_same_currency() {
        let usd = Iso4217::new("USD").unwrap();
        let a = Money::new(Decimal::new(100, 0), usd.clone());
        let b = Money::new(Decimal::new(50, 0), usd);
        let sum = a.try_add(b).expect("same currency should add");
        assert_eq!(sum.amount, Decimal::new(150, 0));
        assert_eq!(sum.currency.0, "USD");
    }

    #[test]
    fn test_money_try_add_different_currencies() {
        let usd = Iso4217::new("USD").unwrap();
        let eur = Iso4217::new("EUR").unwrap();
        let a = Money::new(Decimal::new(100, 0), usd);
        let b = Money::new(Decimal::new(50, 0), eur);
        assert!(a.try_add(b).is_err());
    }

    #[test]
    fn test_money_zero() {
        let usd = Iso4217::new("USD").unwrap();
        let z = Money::zero(usd);
        assert_eq!(z.amount, Decimal::ZERO);
    }

    #[test]
    fn test_time_range_overlaps_true() {
        let a = TimeRange {
            start: Utc.with_ymd_and_hms(2026, 5, 20, 9, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 5, 20, 11, 0, 0).unwrap(),
        };
        let b = TimeRange {
            start: Utc.with_ymd_and_hms(2026, 5, 20, 10, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 5, 20, 12, 0, 0).unwrap(),
        };
        assert!(a.overlaps(&b));
        assert!(b.overlaps(&a));
    }

    #[test]
    fn test_time_range_overlaps_adjacent_is_false() {
        let a = TimeRange {
            start: Utc.with_ymd_and_hms(2026, 5, 20, 9, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 5, 20, 10, 0, 0).unwrap(),
        };
        let b = TimeRange {
            start: Utc.with_ymd_and_hms(2026, 5, 20, 10, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 5, 20, 11, 0, 0).unwrap(),
        };
        assert!(!a.overlaps(&b));
        assert!(!b.overlaps(&a));
    }

    #[test]
    fn test_time_range_overlaps_disjoint() {
        let a = TimeRange {
            start: Utc.with_ymd_and_hms(2026, 5, 20, 9, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 5, 20, 10, 0, 0).unwrap(),
        };
        let b = TimeRange {
            start: Utc.with_ymd_and_hms(2026, 5, 20, 11, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 5, 20, 12, 0, 0).unwrap(),
        };
        assert!(!a.overlaps(&b));
        assert!(!b.overlaps(&a));
    }

    #[test]
    fn test_time_range_is_valid() {
        let good = TimeRange {
            start: Utc.with_ymd_and_hms(2026, 5, 20, 9, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 5, 20, 10, 0, 0).unwrap(),
        };
        assert!(good.is_valid());
        let bad = TimeRange {
            start: Utc.with_ymd_and_hms(2026, 5, 20, 10, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 5, 20, 9, 0, 0).unwrap(),
        };
        assert!(!bad.is_valid());
    }

    #[test]
    fn test_gender_serde_roundtrip() {
        for g in [Gender::Male, Gender::Female, Gender::Other, Gender::Unknown] {
            let json = serde_json::to_string(&g).expect("serialize");
            let back: Gender = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, g);
        }
        // Verify lowercase rename
        assert_eq!(serde_json::to_string(&Gender::Male).unwrap(), "\"male\"");
        assert_eq!(
            serde_json::to_string(&Gender::Female).unwrap(),
            "\"female\""
        );
    }
}
