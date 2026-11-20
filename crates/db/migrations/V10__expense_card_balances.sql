ALTER TABLE expenses
ADD COLUMN payment_document_balance_applied INTEGER NOT NULL DEFAULT 0
CHECK (payment_document_balance_applied IN (0, 1));

CREATE TRIGGER expenses_balance_applied_requires_debit_insert
BEFORE INSERT ON expenses
WHEN NEW.payment_document_balance_applied = 1 AND NOT EXISTS (
    SELECT 1
    FROM payment_documents
    WHERE id = NEW.payment_document_id
      AND card_type = 'debit'
      AND currency = NEW.currency
)
BEGIN
    SELECT RAISE(ABORT, 'balance-applied expense requires a debit card of the same currency');
END;

CREATE TRIGGER expenses_balance_applied_requires_debit_update
BEFORE UPDATE OF payment_document_id, currency, payment_document_balance_applied ON expenses
WHEN NEW.payment_document_balance_applied = 1 AND NOT EXISTS (
    SELECT 1
    FROM payment_documents
    WHERE id = NEW.payment_document_id
      AND card_type = 'debit'
      AND currency = NEW.currency
)
BEGIN
    SELECT RAISE(ABORT, 'balance-applied expense requires a debit card of the same currency');
END;

CREATE TRIGGER payment_documents_preserve_debit_for_applied_expenses
BEFORE UPDATE OF card_type ON payment_documents
WHEN NEW.card_type <> 'debit' AND EXISTS (
    SELECT 1
    FROM expenses
    WHERE payment_document_id = OLD.id
      AND payment_document_balance_applied = 1
)
BEGIN
    SELECT RAISE(ABORT, 'payment document has expenses deducted from its debit balance');
END;