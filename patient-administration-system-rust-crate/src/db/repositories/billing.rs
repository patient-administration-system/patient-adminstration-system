//! billing repository — Account, Charge, Invoice, Payment

use std::str::FromStr;

use rust_decimal::Decimal;
use sea_orm::*;
use uuid::Uuid;

use crate::db::entities::{account, charge, invoice, payment};
use crate::models::billing::{
    Account, AccountStatus, Charge, Invoice, InvoiceStatus, Payment, PaymentMethod,
};
use crate::models::{Iso4217, Money};
use crate::{Error, Result};

pub struct BillingRepository;

impl BillingRepository {
    pub async fn create_account<C: ConnectionTrait>(conn: &C, a: &Account) -> Result<Account> {
        let am = account_to_active_model(a);
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(a.clone())
    }

    pub async fn find_open_account_for_patient<C: ConnectionTrait>(
        conn: &C,
        patient_id: Uuid,
    ) -> Result<Option<Account>> {
        let m = account::Entity::find()
            .filter(account::Column::PatientId.eq(patient_id))
            .filter(account::Column::Status.eq(account_status_to_str(AccountStatus::Open)))
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(account_from_model).transpose()
    }

    pub async fn close_account<C: ConnectionTrait>(conn: &C, id: Uuid) -> Result<Account> {
        let m = account::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("account {id}")))?;
        let now = chrono::Utc::now().fixed_offset();
        let mut am: account::ActiveModel = m.into();
        am.status = Set(account_status_to_str(AccountStatus::Closed).to_string());
        am.closed_at = Set(Some(now));
        am.updated_at = Set(now);
        let updated = am.update(conn).await.map_err(Error::Database)?;
        account_from_model(updated)
    }

    pub async fn create_charge<C: ConnectionTrait>(conn: &C, c: &Charge) -> Result<Charge> {
        let am = charge_to_active_model(c);
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(c.clone())
    }

    /// Locate a charge by its primary id. Used by the v0.19 HL7 v2
    /// outbound publisher (`ChargePosted → DFT^P03`) to chain
    /// charge → account → patient when the outbox payload only
    /// carries `charge_id`.
    pub async fn find_charge_by_id<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
    ) -> Result<Option<Charge>> {
        let m = charge::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(charge_from_model).transpose()
    }

    /// Locate an account by its primary id. Companion to
    /// `find_charge_by_id`.
    pub async fn find_account_by_id<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
    ) -> Result<Option<Account>> {
        let m = account::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(account_from_model).transpose()
    }

    pub async fn list_charges_for_account<C: ConnectionTrait>(
        conn: &C,
        account_id: Uuid,
    ) -> Result<Vec<Charge>> {
        let rows = charge::Entity::find()
            .filter(charge::Column::AccountId.eq(account_id))
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(charge_from_model).collect()
    }

    pub async fn create_invoice<C: ConnectionTrait>(conn: &C, i: &Invoice) -> Result<Invoice> {
        let am = invoice_to_active_model(i)?;
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(i.clone())
    }

    pub async fn find_invoice_by_id<C: ConnectionTrait>(
        conn: &C,
        id: Uuid,
    ) -> Result<Option<Invoice>> {
        let m = invoice::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(Error::Database)?;
        m.map(invoice_from_model).transpose()
    }

    pub async fn update_invoice<C: ConnectionTrait>(conn: &C, i: &Invoice) -> Result<Invoice> {
        let mut am = invoice_to_active_model(i)?;
        am.created_at = NotSet;
        am.update(conn).await.map_err(Error::Database)?;
        Ok(i.clone())
    }

    pub async fn create_payment<C: ConnectionTrait>(conn: &C, p: &Payment) -> Result<Payment> {
        let am = payment_to_active_model(p);
        am.insert(conn).await.map_err(Error::Database)?;
        Ok(p.clone())
    }

    pub async fn list_payments_for_invoice<C: ConnectionTrait>(
        conn: &C,
        invoice_id: Uuid,
    ) -> Result<Vec<Payment>> {
        let rows = payment::Entity::find()
            .filter(payment::Column::InvoiceId.eq(invoice_id))
            .all(conn)
            .await
            .map_err(Error::Database)?;
        rows.into_iter().map(payment_from_model).collect()
    }
}

// --- enum conversions ---

pub(crate) fn account_status_to_str(s: AccountStatus) -> &'static str {
    match s {
        AccountStatus::Open => "open",
        AccountStatus::Closed => "closed",
    }
}

pub(crate) fn account_status_from_str(s: &str) -> Result<AccountStatus> {
    match s {
        "open" => Ok(AccountStatus::Open),
        "closed" => Ok(AccountStatus::Closed),
        other => Err(Error::internal(format!("unknown account status: {other}"))),
    }
}

pub(crate) fn invoice_status_to_str(s: InvoiceStatus) -> &'static str {
    match s {
        InvoiceStatus::Draft => "draft",
        InvoiceStatus::Finalized => "finalized",
        InvoiceStatus::Paid => "paid",
        InvoiceStatus::PartiallyPaid => "partially_paid",
        InvoiceStatus::Void => "void",
    }
}

pub(crate) fn invoice_status_from_str(s: &str) -> Result<InvoiceStatus> {
    match s {
        "draft" => Ok(InvoiceStatus::Draft),
        "finalized" => Ok(InvoiceStatus::Finalized),
        "paid" => Ok(InvoiceStatus::Paid),
        "partially_paid" => Ok(InvoiceStatus::PartiallyPaid),
        "void" => Ok(InvoiceStatus::Void),
        other => Err(Error::internal(format!("unknown invoice status: {other}"))),
    }
}

pub(crate) fn payment_method_to_str(m: PaymentMethod) -> &'static str {
    match m {
        PaymentMethod::Cash => "cash",
        PaymentMethod::Card => "card",
        PaymentMethod::BankTransfer => "bank_transfer",
        PaymentMethod::Insurance => "insurance",
        PaymentMethod::Other => "other",
    }
}

pub(crate) fn payment_method_from_str(s: &str) -> Result<PaymentMethod> {
    match s {
        "cash" => Ok(PaymentMethod::Cash),
        "card" => Ok(PaymentMethod::Card),
        "bank_transfer" => Ok(PaymentMethod::BankTransfer),
        "insurance" => Ok(PaymentMethod::Insurance),
        "other" => Ok(PaymentMethod::Other),
        other => Err(Error::internal(format!("unknown payment method: {other}"))),
    }
}

// --- struct conversions ---

fn account_to_active_model(a: &Account) -> account::ActiveModel {
    account::ActiveModel {
        id: Set(a.id),
        patient_id: Set(a.patient_id),
        status: Set(account_status_to_str(a.status).to_string()),
        currency: Set(a.currency.0.clone()),
        opened_at: Set(a.opened_at.fixed_offset()),
        closed_at: Set(a.closed_at.map(|t| t.fixed_offset())),
        created_at: Set(a.created_at.fixed_offset()),
        updated_at: Set(a.updated_at.fixed_offset()),
    }
}

fn account_from_model(m: account::Model) -> Result<Account> {
    Ok(Account {
        id: m.id,
        patient_id: m.patient_id,
        status: account_status_from_str(&m.status)?,
        currency: Iso4217::new(&m.currency)?,
        opened_at: m.opened_at.with_timezone(&chrono::Utc),
        closed_at: m.closed_at.map(|t| t.with_timezone(&chrono::Utc)),
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

fn charge_to_active_model(c: &Charge) -> charge::ActiveModel {
    charge::ActiveModel {
        id: Set(c.id),
        account_id: Set(c.account_id),
        encounter_id: Set(c.encounter_id),
        appointment_id: Set(c.appointment_id),
        code: Set(c.code.clone()),
        description: Set(c.description.clone()),
        amount_value: Set(c.amount.amount.to_string()),
        amount_currency: Set(c.amount.currency.0.clone()),
        posted_at: Set(c.posted_at.fixed_offset()),
        created_at: Set(c.created_at.fixed_offset()),
    }
}

fn charge_from_model(m: charge::Model) -> Result<Charge> {
    let amount_decimal = Decimal::from_str(&m.amount_value)
        .map_err(|e| Error::internal(format!("parse charge amount: {e}")))?;
    let currency = Iso4217::new(&m.amount_currency)?;
    Ok(Charge {
        id: m.id,
        account_id: m.account_id,
        encounter_id: m.encounter_id,
        appointment_id: m.appointment_id,
        code: m.code,
        description: m.description,
        amount: Money::new(amount_decimal, currency),
        posted_at: m.posted_at.with_timezone(&chrono::Utc),
        created_at: m.created_at.with_timezone(&chrono::Utc),
    })
}

fn invoice_to_active_model(i: &Invoice) -> Result<invoice::ActiveModel> {
    let charge_ids = serde_json::to_value(&i.charge_ids)
        .map_err(|e| Error::internal(format!("serialize charge_ids: {e}")))?;
    Ok(invoice::ActiveModel {
        id: Set(i.id),
        account_id: Set(i.account_id),
        status: Set(invoice_status_to_str(i.status).to_string()),
        total_value: Set(i.total.amount.to_string()),
        total_currency: Set(i.total.currency.0.clone()),
        charge_ids: Set(charge_ids),
        finalized_at: Set(i.finalized_at.map(|t| t.fixed_offset())),
        created_at: Set(i.created_at.fixed_offset()),
        updated_at: Set(i.updated_at.fixed_offset()),
    })
}

fn invoice_from_model(m: invoice::Model) -> Result<Invoice> {
    let amount_decimal = Decimal::from_str(&m.total_value)
        .map_err(|e| Error::internal(format!("parse invoice total: {e}")))?;
    let currency = Iso4217::new(&m.total_currency)?;
    let charge_ids: Vec<Uuid> = serde_json::from_value(m.charge_ids)
        .map_err(|e| Error::internal(format!("deserialize charge_ids: {e}")))?;
    Ok(Invoice {
        id: m.id,
        account_id: m.account_id,
        status: invoice_status_from_str(&m.status)?,
        total: Money::new(amount_decimal, currency),
        charge_ids,
        finalized_at: m.finalized_at.map(|t| t.with_timezone(&chrono::Utc)),
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

fn payment_to_active_model(p: &Payment) -> payment::ActiveModel {
    payment::ActiveModel {
        id: Set(p.id),
        invoice_id: Set(p.invoice_id),
        amount_value: Set(p.amount.amount.to_string()),
        amount_currency: Set(p.amount.currency.0.clone()),
        method: Set(payment_method_to_str(p.method).to_string()),
        reference: Set(p.reference.clone()),
        posted_at: Set(p.posted_at.fixed_offset()),
        created_at: Set(p.created_at.fixed_offset()),
    }
}

fn payment_from_model(m: payment::Model) -> Result<Payment> {
    let amount_decimal = Decimal::from_str(&m.amount_value)
        .map_err(|e| Error::internal(format!("parse payment amount: {e}")))?;
    let currency = Iso4217::new(&m.amount_currency)?;
    Ok(Payment {
        id: m.id,
        invoice_id: m.invoice_id,
        amount: Money::new(amount_decimal, currency),
        method: payment_method_from_str(&m.method)?,
        reference: m.reference,
        posted_at: m.posted_at.with_timezone(&chrono::Utc),
        created_at: m.created_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn usd() -> Iso4217 {
        Iso4217::new("USD").unwrap()
    }

    #[test]
    fn test_invoice_roundtrip_via_active_model() {
        let mut inv = Invoice::new(Uuid::new_v4(), usd());
        inv.total = Money::new(Decimal::new(12345, 2), usd());
        inv.charge_ids = vec![Uuid::new_v4(), Uuid::new_v4()];
        let am = invoice_to_active_model(&inv).expect("to_active_model");
        let m = invoice::Model {
            id: am.id.clone().unwrap(),
            account_id: am.account_id.clone().unwrap(),
            status: am.status.clone().unwrap(),
            total_value: am.total_value.clone().unwrap(),
            total_currency: am.total_currency.clone().unwrap(),
            charge_ids: am.charge_ids.clone().unwrap(),
            finalized_at: am.finalized_at.clone().unwrap(),
            created_at: am.created_at.clone().unwrap(),
            updated_at: am.updated_at.clone().unwrap(),
        };
        let back = invoice_from_model(m).expect("from_model");
        assert_eq!(back.id, inv.id);
        assert_eq!(back.status, InvoiceStatus::Draft);
        assert_eq!(back.total.amount, inv.total.amount);
        assert_eq!(back.total.currency, usd());
        assert_eq!(back.charge_ids.len(), 2);
    }
}
