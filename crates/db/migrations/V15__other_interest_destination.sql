DROP TRIGGER document_rate_links_same_bank_insert;
DROP TRIGGER document_rate_links_same_bank_update;
DROP TRIGGER payment_documents_bank_update;
DROP TRIGGER transfers_source_savings_valid_insert;
DROP TRIGGER transfers_destination_savings_valid_insert;
DROP TRIGGER transfers_endpoints_valid_update;

CREATE TABLE savings_v15 (
    id INTEGER PRIMARY KEY,
    bank_id INTEGER NOT NULL REFERENCES banks(id) ON DELETE RESTRICT,
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    amount TEXT NOT NULL CHECK (
        length(amount) > 0
        AND amount NOT GLOB '*[^0-9.]*'
        AND amount NOT LIKE '.%'
        AND amount NOT LIKE '%.'
        AND amount NOT GLOB '*.*.*'
        AND length(replace(replace(amount, '0', ''), '.', '')) > 0
    ),
    base_rate TEXT NOT NULL CHECK (
        length(base_rate) > 0
        AND base_rate NOT GLOB '*[^0-9.]*'
        AND base_rate NOT LIKE '.%'
        AND base_rate NOT LIKE '%.'
        AND base_rate NOT GLOB '*.*.*'
    ),
    rate_after_threshold TEXT CHECK (
        rate_after_threshold IS NULL OR (
            length(rate_after_threshold) > 0
            AND rate_after_threshold NOT GLOB '*[^0-9.]*'
            AND rate_after_threshold NOT LIKE '.%'
            AND rate_after_threshold NOT LIKE '%.'
            AND rate_after_threshold NOT GLOB '*.*.*'
        )
    ),
    threshold_amount TEXT CHECK (
        threshold_amount IS NULL OR (
            length(threshold_amount) > 0
            AND threshold_amount NOT GLOB '*[^0-9.]*'
            AND threshold_amount NOT LIKE '.%'
            AND threshold_amount NOT LIKE '%.'
            AND threshold_amount NOT GLOB '*.*.*'
            AND length(replace(replace(threshold_amount, '0', ''), '.', '')) > 0
        )
    ),
    term_type TEXT NOT NULL CHECK (term_type IN ('limited', 'unlimited')),
    term_end_date TEXT,
    interest_destination TEXT NOT NULL CHECK (
        interest_destination IN ('reinvest', 'withdraw', 'other')
    ),
    is_active INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    opened_date TEXT CHECK (
        opened_date IS NULL OR (
            length(opened_date) = 10
            AND opened_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'
            AND date(opened_date, '+0 days') IS NOT NULL
            AND date(opened_date, '+0 days') = opened_date
        )
    ),
    interest_accrual_date TEXT CHECK (
        interest_accrual_date IS NULL OR (
            length(interest_accrual_date) = 10
            AND interest_accrual_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'
            AND date(interest_accrual_date, '+0 days') IS NOT NULL
            AND date(interest_accrual_date, '+0 days') = interest_accrual_date
        )
    ),
    currency TEXT REFERENCES currencies(code) ON DELETE RESTRICT,
    CHECK ((rate_after_threshold IS NULL) = (threshold_amount IS NULL)),
    CHECK (
        (term_type = 'limited' AND term_end_date IS NOT NULL AND length(term_end_date) = 10)
        OR (term_type = 'unlimited' AND term_end_date IS NULL)
    )
);

INSERT INTO savings_v15 (
    id, bank_id, title, amount, base_rate, rate_after_threshold, threshold_amount,
    term_type, term_end_date, interest_destination, is_active, opened_date,
    interest_accrual_date, currency
)
SELECT
    id, bank_id, title, amount, base_rate, rate_after_threshold, threshold_amount,
    term_type, term_end_date, interest_destination, is_active, opened_date,
    interest_accrual_date, currency
FROM savings;

DROP TABLE savings;
ALTER TABLE savings_v15 RENAME TO savings;

CREATE INDEX savings_bank_active_title_idx
    ON savings(bank_id, is_active, title COLLATE NOCASE);
CREATE INDEX savings_currency_idx ON savings(currency);

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

CREATE TRIGGER transfers_source_savings_valid_insert
BEFORE INSERT ON transfers
WHEN NEW.source_kind = 'savings' AND NOT EXISTS (
    SELECT 1 FROM savings
    WHERE savings.id = NEW.source_savings_id
      AND savings.currency IS NOT NULL
      AND savings.currency = NEW.currency
)
BEGIN
    SELECT RAISE(ABORT, 'transfer source savings currency must be explicit and match');
END;

CREATE TRIGGER transfers_destination_savings_valid_insert
BEFORE INSERT ON transfers
WHEN NEW.destination_kind = 'savings' AND NOT EXISTS (
    SELECT 1 FROM savings
    WHERE savings.id = NEW.destination_savings_id
      AND savings.currency IS NOT NULL
      AND savings.currency = NEW.currency
)
BEGIN
    SELECT RAISE(ABORT, 'transfer destination savings currency must be explicit and match');
END;

CREATE TRIGGER transfers_endpoints_valid_update
BEFORE UPDATE ON transfers
WHEN (
    NEW.source_kind = 'card' AND NOT EXISTS (
        SELECT 1 FROM payment_documents AS document
        WHERE document.id = NEW.source_payment_document_id
          AND document.card_type = 'debit'
          AND document.currency IS NOT NULL
          AND document.currency = NEW.currency
    )
) OR (
    NEW.destination_kind = 'card' AND NOT EXISTS (
        SELECT 1 FROM payment_documents AS document
        WHERE document.id = NEW.destination_payment_document_id
          AND document.card_type = 'debit'
          AND document.currency IS NOT NULL
          AND document.currency = NEW.currency
    )
) OR (
    NEW.source_kind = 'savings' AND NOT EXISTS (
        SELECT 1 FROM savings
        WHERE savings.id = NEW.source_savings_id
          AND savings.currency IS NOT NULL
          AND savings.currency = NEW.currency
    )
) OR (
    NEW.destination_kind = 'savings' AND NOT EXISTS (
        SELECT 1 FROM savings
        WHERE savings.id = NEW.destination_savings_id
          AND savings.currency IS NOT NULL
          AND savings.currency = NEW.currency
    )
)
BEGIN
    SELECT RAISE(ABORT, 'transfer endpoints must be debit cards or savings with the same explicit currency');
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

CREATE TRIGGER transfers_preserve_savings_currency
BEFORE UPDATE OF currency ON savings
WHEN NEW.currency IS NOT OLD.currency AND EXISTS (
    SELECT 1 FROM transfers
    WHERE source_savings_id = OLD.id
       OR destination_savings_id = OLD.id
)
BEGIN
    SELECT RAISE(ABORT, 'savings currency cannot change while it is used in transfers');
END;