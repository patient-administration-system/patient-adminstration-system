//! billing — Account/Charge/Invoice/Payment service

use sea_orm::{DatabaseConnection, TransactionTrait};
use std::sync::Arc;
use uuid::Uuid;

use crate::db::repositories::{
    audit::{AuditLogRepository, UserContext},
    billing::BillingRepository,
    outbox::OutboxRepository,
};
use crate::models::billing::{Account, Charge, Invoice, InvoiceStatus, Payment, PaymentMethod};
use crate::models::{Iso4217, Money};
use crate::streaming::{DomainEvent, EventPublisher};
use crate::validation::validate_charge;
use crate::{Error, Result};

pub struct BillingService {
    db: DatabaseConnection,
    publisher: Arc<dyn EventPublisher>,
}

/// Bundle of fields for posting a charge. Avoids a >7-argument service call.
#[derive(Debug, Clone)]
pub struct PostChargeInput {
    pub account_id: Uuid,
    pub code: String,
    pub description: String,
    pub amount: Money,
    pub encounter_id: Option<Uuid>,
    pub appointment_id: Option<Uuid>,
}

impl BillingService {
    pub fn new(db: DatabaseConnection, publisher: Arc<dyn EventPublisher>) -> Self {
        Self { db, publisher }
    }

    pub async fn open_account(
        &self,
        patient_id: Uuid,
        currency: Iso4217,
        ctx: &UserContext,
    ) -> Result<Account> {
        if let Some(_existing) =
            BillingRepository::find_open_account_for_patient(&self.db, patient_id).await?
        {
            return Err(Error::conflict(format!(
                "patient {patient_id} already has an open account"
            )));
        }
        let ctx_clone = ctx.clone();
        let account = Account::new(patient_id, currency);
        let account_clone = account.clone();
        let res = self
            .db
            .transaction::<_, Account, Error>(|txn| {
                Box::pin(async move {
                    let a = BillingRepository::create_account(txn, &account_clone).await?;
                    AuditLogRepository::log(
                        txn,
                        "account",
                        a.id,
                        "open",
                        None,
                        Some(serde_json::to_value(&a).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "AccountOpened",
                        &serde_json::json!({ "account_id": a.id, "patient_id": patient_id }),
                    )
                    .await?;
                    Ok(a)
                })
            })
            .await
            .map_err(unwrap_txn_err)?;
        let _ = self
            .publisher
            .publish(DomainEvent::new(
                "AccountOpened",
                serde_json::json!({ "account_id": res.id }),
            ))
            .await;
        Ok(res)
    }

    pub async fn close_account(&self, account_id: Uuid, ctx: &UserContext) -> Result<Account> {
        let ctx_clone = ctx.clone();
        let res = self
            .db
            .transaction::<_, Account, Error>(|txn| {
                Box::pin(async move {
                    let a = BillingRepository::close_account(txn, account_id).await?;
                    AuditLogRepository::log(
                        txn, "account", account_id, "close", None, None, &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "AccountClosed",
                        &serde_json::json!({ "account_id": account_id }),
                    )
                    .await?;
                    Ok(a)
                })
            })
            .await
            .map_err(unwrap_txn_err)?;
        Ok(res)
    }

    pub async fn post_charge(&self, req: PostChargeInput, ctx: &UserContext) -> Result<Charge> {
        let PostChargeInput {
            account_id,
            code,
            description,
            amount,
            encounter_id,
            appointment_id,
        } = req;
        let mut charge = Charge::new(account_id, code, description, amount);
        charge.encounter_id = encounter_id;
        charge.appointment_id = appointment_id;
        validate_charge(&charge)?;
        let ctx_clone = ctx.clone();
        let charge_clone = charge.clone();
        let res = self
            .db
            .transaction::<_, Charge, Error>(|txn| {
                Box::pin(async move {
                    let c = BillingRepository::create_charge(txn, &charge_clone).await?;
                    AuditLogRepository::log(
                        txn,
                        "charge",
                        c.id,
                        "post",
                        None,
                        Some(serde_json::to_value(&c).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "ChargePosted",
                        &serde_json::json!({ "charge_id": c.id, "account_id": account_id }),
                    )
                    .await?;
                    Ok(c)
                })
            })
            .await
            .map_err(unwrap_txn_err)?;
        Ok(res)
    }

    pub async fn finalize_invoice(
        &self,
        account_id: Uuid,
        charge_ids: Vec<Uuid>,
        ctx: &UserContext,
    ) -> Result<Invoice> {
        let all_charges = BillingRepository::list_charges_for_account(&self.db, account_id).await?;
        let selected: Vec<Charge> = all_charges
            .into_iter()
            .filter(|c| charge_ids.contains(&c.id))
            .collect();
        if selected.is_empty() {
            return Err(Error::validation(
                "no matching charges found for invoice finalization",
            ));
        }
        let currency = selected[0].amount.currency.clone();
        let mut total = Money::zero(currency.clone());
        for c in &selected {
            total = total.try_add(c.amount.clone())?;
        }
        let mut invoice = Invoice::new(account_id, currency);
        invoice.charge_ids = charge_ids.clone();
        invoice.total = total;
        invoice.finalize()?;
        let ctx_clone = ctx.clone();
        let invoice_clone = invoice.clone();
        let res = self
            .db
            .transaction::<_, Invoice, Error>(|txn| {
                Box::pin(async move {
                    let i = BillingRepository::create_invoice(txn, &invoice_clone).await?;
                    AuditLogRepository::log(
                        txn,
                        "invoice",
                        i.id,
                        "finalize",
                        None,
                        Some(serde_json::to_value(&i).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "InvoiceFinalized",
                        &serde_json::json!({ "invoice_id": i.id }),
                    )
                    .await?;
                    Ok(i)
                })
            })
            .await
            .map_err(unwrap_txn_err)?;
        Ok(res)
    }

    pub async fn post_payment(
        &self,
        invoice_id: Uuid,
        amount: Money,
        method: PaymentMethod,
        reference: Option<String>,
        ctx: &UserContext,
    ) -> Result<Payment> {
        let mut payment = Payment::new(invoice_id, amount.clone(), method);
        payment.reference = reference;
        let ctx_clone = ctx.clone();
        let payment_clone = payment.clone();
        let res = self
            .db
            .transaction::<_, Payment, Error>(|txn| {
                Box::pin(async move {
                    let p = BillingRepository::create_payment(txn, &payment_clone).await?;
                    let invoice = BillingRepository::find_invoice_by_id(txn, invoice_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("invoice {invoice_id}")))?;
                    let payments =
                        BillingRepository::list_payments_for_invoice(txn, invoice_id).await?;
                    let mut paid = Money::zero(invoice.total.currency.clone());
                    for pmt in &payments {
                        paid = paid.try_add(pmt.amount.clone())?;
                    }
                    let mut next = invoice.clone();
                    next.status = if paid.amount >= invoice.total.amount {
                        InvoiceStatus::Paid
                    } else {
                        InvoiceStatus::PartiallyPaid
                    };
                    next.updated_at = chrono::Utc::now();
                    BillingRepository::update_invoice(txn, &next).await?;
                    AuditLogRepository::log(
                        txn,
                        "payment",
                        p.id,
                        "post",
                        None,
                        Some(serde_json::to_value(&p).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "PaymentPosted",
                        &serde_json::json!({ "payment_id": p.id, "invoice_id": invoice_id }),
                    )
                    .await?;
                    Ok(p)
                })
            })
            .await
            .map_err(unwrap_txn_err)?;
        Ok(res)
    }
}

fn unwrap_txn_err(e: sea_orm::TransactionError<Error>) -> Error {
    match e {
        sea_orm::TransactionError::Connection(c) => Error::Database(c),
        sea_orm::TransactionError::Transaction(t) => t,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_money_sum_smoke() {
        let usd = Iso4217::new("USD").unwrap();
        let a = Money::new(rust_decimal::Decimal::new(100, 2), usd.clone());
        let b = Money::new(rust_decimal::Decimal::new(200, 2), usd);
        let s = a.try_add(b).unwrap();
        assert_eq!(s.amount, rust_decimal::Decimal::new(300, 2));
    }
}
