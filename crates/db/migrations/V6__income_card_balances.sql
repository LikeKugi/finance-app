CREATE TEMP TABLE migration_document_rate_links AS
SELECT id, document_id, savings_id FROM document_rate_links;

DROP TRIGGER savings_bank_update;
DROP TABLE document_rate_links;

CREATE TABLE payment_documents_v6 (
    id INTEGER PRIMARY KEY,
    bank_id INTEGER NOT NULL REFERENCES banks(id) ON DELETE RESTRICT,
    payment_system TEXT NOT NULL CHECK (payment_system IN ('VISA', 'MasterCard', 'MIR')),
    card_type TEXT NOT NULL CHECK (card_type IN ('debit', 'credit')),
    amount TEXT NOT NULL CHECK (
        length(amount) > 0
        AND amount NOT GLOB '*[^0-9.]*'
        AND amount NOT LIKE '.%'
        AND amount NOT LIKE '%.'
        AND amount NOT GLOB '*.*.*'
        AND (
            amount = '0'
            OR length(replace(replace(amount, '0', ''), '.', '')) > 0
        )
    ),
    currency TEXT CHECK (currency IS NULL OR currency IN ('RUB', 'USD')),
    due_date TEXT,
    grace_period_days INTEGER,
    affects_deposit_rate INTEGER NOT NULL DEFAULT 0 CHECK (affects_deposit_rate IN (0, 1)),
    min_monthly_spend TEXT CHECK (
        min_monthly_spend IS NULL OR (
            length(min_monthly_spend) > 0
            AND min_monthly_spend NOT GLOB '*[^0-9.]*'
            AND min_monthly_spend NOT LIKE '.%'
            AND min_monthly_spend NOT LIKE '%.'
            AND min_monthly_spend NOT GLOB '*.*.*'
            AND length(replace(replace(min_monthly_spend, '0', ''), '.', '')) > 0
        )
    ),
    CHECK (
        (card_type = 'debit' AND due_date IS NULL AND grace_period_days IS NULL)
        OR (
            card_type = 'credit'
            AND amount <> '0'
            AND due_date IS NOT NULL
            AND length(due_date) = 10
            AND date(due_date) = due_date
            AND grace_period_days IS NOT NULL
            AND grace_period_days >= 0
        )
    ),
    CHECK (
        (affects_deposit_rate = 1 AND min_monthly_spend IS NOT NULL)
        OR (affects_deposit_rate = 0 AND min_monthly_spend IS NULL)
    )
);

INSERT INTO payment_documents_v6 (
    id, bank_id, payment_system, card_type, amount, due_date,
    grace_period_days, affects_deposit_rate, min_monthly_spend
)
SELECT
    id, bank_id, payment_system, card_type, amount, due_date,
    grace_period_days, affects_deposit_rate, min_monthly_spend
FROM payment_documents;

DROP TABLE payment_documents;
ALTER TABLE payment_documents_v6 RENAME TO payment_documents;

CREATE TABLE document_rate_links (
    id INTEGER PRIMARY KEY,
    document_id INTEGER NOT NULL REFERENCES payment_documents(id) ON DELETE CASCADE,
    savings_id INTEGER NOT NULL REFERENCES savings(id) ON DELETE CASCADE,
    UNIQUE (document_id, savings_id)
);

INSERT INTO document_rate_links (id, document_id, savings_id)
SELECT id, document_id, savings_id FROM migration_document_rate_links;
DROP TABLE migration_document_rate_links;

CREATE INDEX payment_documents_bank_type_system_idx
    ON payment_documents(bank_id, card_type, payment_system, id);
CREATE INDEX document_rate_links_savings_idx ON document_rate_links(savings_id);

CREATE TRIGGER document_rate_links_same_bank_insert
BEFORE INSERT ON document_rate_links
WHEN NOT EXISTS (
    SELECT 1
    FROM payment_documents AS document
    JOIN savings ON savings.id = NEW.savings_id
    WHERE document.id = NEW.document_id
      AND document.bank_id = savings.bank_id
      AND document.affects_deposit_rate = 1
)
BEGIN
    SELECT RAISE(ABORT, 'card and savings must belong to the same bank and card must affect rates');
END;

CREATE TRIGGER document_rate_links_same_bank_update
BEFORE UPDATE OF document_id, savings_id ON document_rate_links
WHEN NOT EXISTS (
    SELECT 1
    FROM payment_documents AS document
    JOIN savings ON savings.id = NEW.savings_id
    WHERE document.id = NEW.document_id
      AND document.bank_id = savings.bank_id
      AND document.affects_deposit_rate = 1
)
BEGIN
    SELECT RAISE(ABORT, 'card and savings must belong to the same bank and card must affect rates');
END;

CREATE TRIGGER payment_documents_bank_update
BEFORE UPDATE OF bank_id ON payment_documents
WHEN EXISTS (
    SELECT 1
    FROM document_rate_links AS link
    JOIN savings ON savings.id = link.savings_id
    WHERE link.document_id = OLD.id
      AND savings.bank_id <> NEW.bank_id
)
BEGIN
    SELECT RAISE(ABORT, 'card and savings must belong to the same bank');
END;

CREATE TRIGGER savings_bank_update
BEFORE UPDATE OF bank_id ON savings
WHEN EXISTS (
    SELECT 1
    FROM document_rate_links AS link
    JOIN payment_documents AS document ON document.id = link.document_id
    WHERE link.savings_id = OLD.id
      AND document.bank_id <> NEW.bank_id
)
BEGIN
    SELECT RAISE(ABORT, 'card and savings must belong to the same bank');
END;

CREATE TRIGGER payment_documents_disable_rate_with_links
BEFORE UPDATE OF affects_deposit_rate ON payment_documents
WHEN NEW.affects_deposit_rate = 0 AND EXISTS (
    SELECT 1 FROM document_rate_links WHERE document_id = OLD.id
)
BEGIN
    SELECT RAISE(ABORT, 'remove savings links before disabling rate influence');
END;

ALTER TABLE incomes
ADD COLUMN payment_document_id INTEGER REFERENCES payment_documents(id) ON DELETE RESTRICT;

CREATE INDEX incomes_payment_document_idx ON incomes(payment_document_id);

CREATE TRIGGER incomes_payment_document_insert
BEFORE INSERT ON incomes
WHEN NEW.payment_document_id IS NOT NULL AND NOT EXISTS (
    SELECT 1
    FROM payment_documents
    WHERE id = NEW.payment_document_id
      AND card_type = 'debit'
      AND currency = NEW.currency
)
BEGIN
    SELECT RAISE(ABORT, 'income payment document must be a debit card of the same currency');
END;

CREATE TRIGGER incomes_payment_document_update
BEFORE UPDATE OF payment_document_id, currency ON incomes
WHEN NEW.payment_document_id IS NOT NULL AND NOT EXISTS (
    SELECT 1
    FROM payment_documents
    WHERE id = NEW.payment_document_id
      AND card_type = 'debit'
      AND currency = NEW.currency
)
BEGIN
    SELECT RAISE(ABORT, 'income payment document must be a debit card of the same currency');
END;

CREATE TRIGGER payment_documents_preserve_income_links
BEFORE UPDATE OF card_type, currency ON payment_documents
WHEN EXISTS (
    SELECT 1
    FROM incomes
    WHERE payment_document_id = OLD.id
      AND (NEW.card_type <> 'debit' OR NEW.currency IS NOT currency)
)
BEGIN
    SELECT RAISE(ABORT, 'payment document type and currency must match linked incomes');
END;