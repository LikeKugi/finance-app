DROP TRIGGER document_rate_links_same_bank_insert;
DROP TRIGGER document_rate_links_same_bank_update;
DROP TRIGGER savings_bank_update;
DROP TRIGGER incomes_payment_document_insert;
DROP TRIGGER incomes_payment_document_update;
DROP TRIGGER expenses_payment_document_insert;
DROP TRIGGER expenses_payment_document_update;
DROP TRIGGER expenses_rate_requires_qualifying_card_insert;
DROP TRIGGER expenses_rate_requires_qualifying_card_update;

CREATE TABLE currencies (
    code TEXT PRIMARY KEY NOT NULL CHECK (
        length(code) = 3 AND code NOT GLOB '*[^A-Z]*'
    ),
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    is_primary INTEGER NOT NULL DEFAULT 0 CHECK (is_primary IN (0, 1)),
    rate_to_primary TEXT CHECK (
        rate_to_primary IS NULL OR (
            length(rate_to_primary) > 0
            AND rate_to_primary NOT GLOB '*[^0-9.]*'
            AND rate_to_primary NOT LIKE '.%'
            AND rate_to_primary NOT LIKE '%.'
            AND rate_to_primary NOT GLOB '*.*.*'
            AND length(replace(replace(rate_to_primary, '0', ''), '.', '')) > 0
        )
    ),
    rate_date TEXT CHECK (
        rate_date IS NULL OR (length(rate_date) = 10 AND date(rate_date) = rate_date)
    ),
    CHECK ((rate_to_primary IS NULL) = (rate_date IS NULL)),
    CHECK (is_primary = 0 OR rate_to_primary IS NULL)
);

CREATE UNIQUE INDEX currencies_single_primary_idx
    ON currencies(is_primary) WHERE is_primary = 1;

CREATE TRIGGER currencies_preserve_primary_delete
BEFORE DELETE ON currencies
WHEN OLD.is_primary = 1
BEGIN
    SELECT RAISE(ABORT, 'cannot delete the primary currency');
END;

INSERT INTO currencies (code, name, is_primary)
VALUES ('RUB', 'Российский рубль', 1), ('USD', 'Доллар США', 0);

CREATE TABLE payment_documents_v8 (
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
    currency TEXT REFERENCES currencies(code) ON DELETE RESTRICT,
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

INSERT INTO payment_documents_v8 (
    id, bank_id, payment_system, card_type, amount, currency, due_date,
    grace_period_days, affects_deposit_rate, min_monthly_spend
)
SELECT
    id, bank_id, payment_system, card_type, amount, currency, due_date,
    grace_period_days, affects_deposit_rate, min_monthly_spend
FROM payment_documents;

DROP TABLE payment_documents;
ALTER TABLE payment_documents_v8 RENAME TO payment_documents;

CREATE TABLE incomes_v8 (
    id INTEGER PRIMARY KEY,
    date TEXT NOT NULL CHECK (length(date) = 10),
    category_id INTEGER NOT NULL REFERENCES income_categories(id) ON DELETE RESTRICT,
    amount TEXT NOT NULL CHECK (
        length(amount) > 0
        AND amount NOT GLOB '*[^0-9.]*'
        AND amount NOT LIKE '.%'
        AND amount NOT LIKE '%.'
        AND amount NOT GLOB '*.*.*'
        AND length(replace(replace(amount, '0', ''), '.', '')) > 0
    ),
    currency TEXT NOT NULL REFERENCES currencies(code) ON DELETE RESTRICT,
    comment TEXT NOT NULL DEFAULT '',
    payment_document_id INTEGER REFERENCES payment_documents(id) ON DELETE RESTRICT
);

INSERT INTO incomes_v8 (id, date, category_id, amount, currency, comment, payment_document_id)
SELECT id, date, category_id, amount, currency, comment, payment_document_id FROM incomes;

DROP TABLE incomes;
ALTER TABLE incomes_v8 RENAME TO incomes;

CREATE TABLE expenses_v8 (
    id INTEGER PRIMARY KEY,
    date TEXT NOT NULL CHECK (length(date) = 10),
    category_id INTEGER NOT NULL REFERENCES expense_categories(id) ON DELETE RESTRICT,
    amount TEXT NOT NULL CHECK (
        length(amount) > 0
        AND amount NOT GLOB '*[^0-9.]*'
        AND amount NOT LIKE '.%'
        AND amount NOT LIKE '%.'
        AND amount NOT GLOB '*.*.*'
        AND length(replace(replace(amount, '0', ''), '.', '')) > 0
    ),
    currency TEXT NOT NULL REFERENCES currencies(code) ON DELETE RESTRICT,
    comment TEXT NOT NULL DEFAULT '',
    payment_document_id INTEGER REFERENCES payment_documents(id) ON DELETE RESTRICT,
    affects_deposit_rate INTEGER NOT NULL DEFAULT 0 CHECK (affects_deposit_rate IN (0, 1))
);

INSERT INTO expenses_v8 (
    id, date, category_id, amount, currency, comment, payment_document_id, affects_deposit_rate
)
SELECT
    id, date, category_id, amount, currency, comment, payment_document_id, affects_deposit_rate
FROM expenses;

DROP TABLE expenses;
ALTER TABLE expenses_v8 RENAME TO expenses;

CREATE INDEX payment_documents_bank_type_system_idx
    ON payment_documents(bank_id, card_type, payment_system, id);
CREATE INDEX incomes_date_id_idx ON incomes(date DESC, id DESC);
CREATE INDEX incomes_category_id_idx ON incomes(category_id);
CREATE INDEX incomes_payment_document_idx ON incomes(payment_document_id);
CREATE INDEX expenses_date_id_idx ON expenses(date DESC, id DESC);
CREATE INDEX expenses_category_id_idx ON expenses(category_id);
CREATE INDEX expenses_payment_document_idx ON expenses(payment_document_id);

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

CREATE TRIGGER payment_documents_disable_rate_with_links
BEFORE UPDATE OF affects_deposit_rate ON payment_documents
WHEN NEW.affects_deposit_rate = 0 AND EXISTS (
    SELECT 1 FROM document_rate_links WHERE document_id = OLD.id
)
BEGIN
    SELECT RAISE(ABORT, 'remove savings links before disabling rate influence');
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