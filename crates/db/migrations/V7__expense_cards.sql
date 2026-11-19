ALTER TABLE expenses
ADD COLUMN payment_document_id INTEGER REFERENCES payment_documents(id) ON DELETE RESTRICT;

ALTER TABLE expenses
ADD COLUMN affects_deposit_rate INTEGER NOT NULL DEFAULT 0 CHECK (affects_deposit_rate IN (0, 1));

CREATE INDEX expenses_payment_document_idx ON expenses(payment_document_id);

CREATE TRIGGER expenses_payment_document_insert
BEFORE INSERT ON expenses
WHEN NEW.payment_document_id IS NOT NULL AND NOT EXISTS (
    SELECT 1
    FROM payment_documents
    WHERE id = NEW.payment_document_id
      AND currency = NEW.currency
)
BEGIN
    SELECT RAISE(ABORT, 'expense payment document must have the same known currency');
END;

CREATE TRIGGER expenses_payment_document_update
BEFORE UPDATE OF payment_document_id, currency ON expenses
WHEN NEW.payment_document_id IS NOT NULL AND NOT EXISTS (
    SELECT 1
    FROM payment_documents
    WHERE id = NEW.payment_document_id
      AND currency = NEW.currency
)
BEGIN
    SELECT RAISE(ABORT, 'expense payment document must have the same known currency');
END;

CREATE TRIGGER expenses_rate_requires_qualifying_card_insert
BEFORE INSERT ON expenses
WHEN NEW.affects_deposit_rate = 1 AND NOT EXISTS (
    SELECT 1
    FROM payment_documents
    WHERE id = NEW.payment_document_id
      AND currency = NEW.currency
      AND affects_deposit_rate = 1
)
BEGIN
    SELECT RAISE(ABORT, 'rate-counted expense requires a qualifying card of the same currency');
END;

CREATE TRIGGER expenses_rate_requires_qualifying_card_update
BEFORE UPDATE OF payment_document_id, currency, affects_deposit_rate ON expenses
WHEN NEW.affects_deposit_rate = 1 AND NOT EXISTS (
    SELECT 1
    FROM payment_documents
    WHERE id = NEW.payment_document_id
      AND currency = NEW.currency
      AND affects_deposit_rate = 1
)
BEGIN
    SELECT RAISE(ABORT, 'rate-counted expense requires a qualifying card of the same currency');
END;

CREATE TRIGGER payment_documents_preserve_expense_links
BEFORE UPDATE OF currency, affects_deposit_rate ON payment_documents
WHEN EXISTS (
    SELECT 1
    FROM expenses
    WHERE payment_document_id = OLD.id
      AND (
          currency IS NOT NEW.currency
          OR (affects_deposit_rate = 1 AND NEW.affects_deposit_rate = 0)
      )
)
BEGIN
    SELECT RAISE(ABORT, 'payment document currency and rate settings must match linked expenses');
END;