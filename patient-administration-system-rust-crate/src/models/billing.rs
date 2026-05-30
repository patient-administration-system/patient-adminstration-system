//! Billing models: Account, Charge, Invoice, Payment.
//!
//! Episode-of-care financials. Money values use `rust_decimal::Decimal` via
//! the shared [`crate::models::Money`] type. Invoices follow a small state
//! machine: `Draft → Finalized → Paid | PartiallyPaid | Void`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Lifecycle state of a patient billing [`Account`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    /// Account is open and can receive new charges.
    Open,
    /// Account is closed; no further charges may be posted.
    Closed,
}

/// Lifecycle state of an [`Invoice`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvoiceStatus {
    /// Invoice is being assembled; charges may be added.
    Draft,
    /// Invoice is final; charges are immutable.
    Finalized,
    /// Invoice has been paid in full.
    Paid,
    /// Invoice has been partially paid.
    PartiallyPaid,
    /// Invoice was voided and is no longer collectible.
    Void,
}

/// Means by which a [`Payment`] was tendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentMethod {
    /// Cash tender.
    Cash,
    /// Credit/debit card.
    Card,
    /// Bank/wire transfer.
    BankTransfer,
    /// Insurance payment.
    Insurance,
    /// Any other payment method.
    Other,
}

/// A patient billing account.
///
/// One open account per patient at a time; a new account opens after the
/// previous one is closed. Currency is fixed at account creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    /// Internal account identifier.
    pub id: Uuid,
    /// Patient who owns the account.
    pub patient_id: Uuid,
    /// Current lifecycle status.
    pub status: AccountStatus,
    /// ISO 4217 currency of all charges/invoices on this account.
    pub currency: crate::models::Iso4217,
    /// When the account was opened.
    pub opened_at: DateTime<Utc>,
    /// When the account was closed, if applicable.
    pub closed_at: Option<DateTime<Utc>>,
    /// Row creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
}

impl Account {
    /// Create a new open account for `patient_id` in `currency`.
    pub fn new(patient_id: Uuid, currency: crate::models::Iso4217) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            patient_id,
            status: AccountStatus::Open,
            currency,
            opened_at: now,
            closed_at: None,
            created_at: now,
            updated_at: now,
        }
    }
}

/// A single financial charge posted against an [`Account`].
///
/// Charges may optionally reference the originating encounter and/or
/// appointment. Amounts are externally supplied — no pricing engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Charge {
    /// Internal charge identifier.
    pub id: Uuid,
    /// Account against which the charge is posted.
    pub account_id: Uuid,
    /// Originating encounter, if any.
    pub encounter_id: Option<Uuid>,
    /// Originating appointment, if any.
    pub appointment_id: Option<Uuid>,
    /// Billing code (CPT/HCPCS/local).
    pub code: String,
    /// Human-readable description of the charge.
    pub description: String,
    /// Amount and currency of the charge.
    pub amount: crate::models::Money,
    /// When the charge was posted.
    pub posted_at: DateTime<Utc>,
    /// Row creation timestamp.
    pub created_at: DateTime<Utc>,
}

impl Charge {
    /// Create a new charge with `posted_at` set to now.
    pub fn new(
        account_id: Uuid,
        code: String,
        description: String,
        amount: crate::models::Money,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            account_id,
            encounter_id: None,
            appointment_id: None,
            code,
            description,
            amount,
            posted_at: now,
            created_at: now,
        }
    }
}

/// A snapshot of charges presented to the patient for payment.
///
/// `total` is the sum of the referenced charges; it is set by the billing
/// service when charges are attached and frozen at finalization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invoice {
    /// Internal invoice identifier.
    pub id: Uuid,
    /// Account the invoice belongs to.
    pub account_id: Uuid,
    /// Current lifecycle status.
    pub status: InvoiceStatus,
    /// Sum of charges (matching the account's currency).
    pub total: crate::models::Money,
    /// Charges included in this invoice.
    pub charge_ids: Vec<Uuid>,
    /// When the invoice was finalized, if it has been.
    pub finalized_at: Option<DateTime<Utc>>,
    /// Row creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
}

impl Invoice {
    /// Create a new draft invoice for `account_id` with zero `total` in `currency`.
    pub fn new(account_id: Uuid, currency: crate::models::Iso4217) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            account_id,
            status: InvoiceStatus::Draft,
            total: crate::models::Money::zero(currency),
            charge_ids: Vec::new(),
            finalized_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Transition this invoice from `Draft` to `Finalized`.
    ///
    /// Returns `Error::InvalidStateTransition` if the invoice is not currently in `Draft`.
    pub fn finalize(&mut self) -> crate::Result<()> {
        if self.status != InvoiceStatus::Draft {
            return Err(crate::Error::invalid_transition(
                "Invoice: must be Draft to finalize",
            ));
        }
        let now = Utc::now();
        self.status = InvoiceStatus::Finalized;
        self.finalized_at = Some(now);
        self.updated_at = now;
        Ok(())
    }
}

/// A payment posted against an [`Invoice`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payment {
    /// Internal payment identifier.
    pub id: Uuid,
    /// Invoice the payment is applied to.
    pub invoice_id: Uuid,
    /// Amount tendered.
    pub amount: crate::models::Money,
    /// How the payment was tendered.
    pub method: PaymentMethod,
    /// External reference (e.g., transaction id, check number).
    pub reference: Option<String>,
    /// When the payment was posted.
    pub posted_at: DateTime<Utc>,
    /// Row creation timestamp.
    pub created_at: DateTime<Utc>,
}

impl Payment {
    /// Create a new payment with `posted_at` set to now and no external `reference`.
    pub fn new(invoice_id: Uuid, amount: crate::models::Money, method: PaymentMethod) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            invoice_id,
            amount,
            method,
            reference: None,
            posted_at: now,
            created_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Iso4217, Money};
    use rust_decimal::Decimal;

    fn usd() -> Iso4217 {
        Iso4217::new("USD").unwrap()
    }

    #[test]
    fn test_account_new_defaults() {
        let patient_id = Uuid::new_v4();
        let acct = Account::new(patient_id, usd());
        assert_eq!(acct.patient_id, patient_id);
        assert_eq!(acct.status, AccountStatus::Open);
        assert_eq!(acct.currency.0, "USD");
        assert!(acct.closed_at.is_none());
        assert_eq!(acct.opened_at, acct.created_at);
        assert_eq!(acct.created_at, acct.updated_at);
    }

    #[test]
    fn test_charge_new_posts_with_amount() {
        let account_id = Uuid::new_v4();
        let amount = Money::new(Decimal::new(12345, 2), usd());
        let charge = Charge::new(
            account_id,
            "99213".to_string(),
            "Office visit, established patient".to_string(),
            amount.clone(),
        );
        assert_eq!(charge.account_id, account_id);
        assert_eq!(charge.code, "99213");
        assert_eq!(charge.description, "Office visit, established patient");
        assert_eq!(charge.amount.amount, amount.amount);
        assert_eq!(charge.amount.currency, amount.currency);
        assert!(charge.encounter_id.is_none());
        assert!(charge.appointment_id.is_none());
        assert_eq!(charge.posted_at, charge.created_at);
    }

    #[test]
    fn test_invoice_new_total_is_zero_in_currency() {
        let account_id = Uuid::new_v4();
        let inv = Invoice::new(account_id, usd());
        assert_eq!(inv.account_id, account_id);
        assert_eq!(inv.status, InvoiceStatus::Draft);
        assert_eq!(inv.total.amount, Decimal::ZERO);
        assert_eq!(inv.total.currency.0, "USD");
        assert!(inv.charge_ids.is_empty());
        assert!(inv.finalized_at.is_none());
    }

    #[test]
    fn test_invoice_finalize_succeeds_from_draft_then_errors() {
        let mut inv = Invoice::new(Uuid::new_v4(), usd());
        assert_eq!(inv.status, InvoiceStatus::Draft);

        inv.finalize().expect("finalize from Draft should succeed");
        assert_eq!(inv.status, InvoiceStatus::Finalized);
        assert!(inv.finalized_at.is_some());

        // Second finalize should fail because status is no longer Draft.
        let err = inv.finalize().expect_err("second finalize should error");
        match err {
            crate::Error::InvalidStateTransition(msg) => {
                assert!(
                    msg.contains("Draft"),
                    "expected message to mention Draft, got: {msg}"
                );
            }
            other => panic!("expected InvalidStateTransition, got {other:?}"),
        }
    }

    #[test]
    fn test_invoice_finalize_errors_from_non_draft_statuses() {
        for status in [
            InvoiceStatus::Finalized,
            InvoiceStatus::Paid,
            InvoiceStatus::PartiallyPaid,
            InvoiceStatus::Void,
        ] {
            let mut inv = Invoice::new(Uuid::new_v4(), usd());
            inv.status = status;
            assert!(inv.finalize().is_err(), "should fail from {:?}", status);
        }
    }

    #[test]
    fn test_payment_new_defaults() {
        let invoice_id = Uuid::new_v4();
        let amount = Money::new(Decimal::new(5000, 2), usd());
        let pay = Payment::new(invoice_id, amount.clone(), PaymentMethod::Card);
        assert_eq!(pay.invoice_id, invoice_id);
        assert_eq!(pay.amount.amount, amount.amount);
        assert_eq!(pay.method, PaymentMethod::Card);
        assert!(pay.reference.is_none());
        assert_eq!(pay.posted_at, pay.created_at);
    }

    #[test]
    fn test_account_serde_roundtrip() {
        let acct = Account::new(Uuid::new_v4(), usd());
        let json = serde_json::to_string(&acct).expect("serialize");
        let back: Account = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, acct.id);
        assert_eq!(back.patient_id, acct.patient_id);
        assert_eq!(back.status, acct.status);
        assert_eq!(back.currency, acct.currency);
        assert_eq!(back.closed_at, acct.closed_at);
        // Confirm snake_case rendering for status.
        assert!(json.contains("\"status\":\"open\""));
    }

    #[test]
    fn test_invoice_serde_roundtrip() {
        let mut inv = Invoice::new(Uuid::new_v4(), usd());
        inv.charge_ids.push(Uuid::new_v4());
        let json = serde_json::to_string(&inv).expect("serialize");
        let back: Invoice = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, inv.id);
        assert_eq!(back.account_id, inv.account_id);
        assert_eq!(back.status, inv.status);
        assert_eq!(back.total.amount, inv.total.amount);
        assert_eq!(back.total.currency, inv.total.currency);
        assert_eq!(back.charge_ids, inv.charge_ids);
        assert_eq!(back.finalized_at, inv.finalized_at);
        // Confirm snake_case rendering for status.
        assert!(json.contains("\"status\":\"draft\""));
    }

    #[test]
    fn test_enum_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&AccountStatus::Open).unwrap(),
            "\"open\""
        );
        assert_eq!(
            serde_json::to_string(&AccountStatus::Closed).unwrap(),
            "\"closed\""
        );
        assert_eq!(
            serde_json::to_string(&InvoiceStatus::PartiallyPaid).unwrap(),
            "\"partially_paid\""
        );
        assert_eq!(
            serde_json::to_string(&PaymentMethod::BankTransfer).unwrap(),
            "\"bank_transfer\""
        );
    }
}
