CREATE TABLE transfers (
    id INTEGER PRIMARY KEY,
    operation_date TEXT NOT NULL CHECK (
        length(operation_date) = 10
        AND date(operation_date) = operation_date
    ),
    operation_type TEXT NOT NULL CHECK (
        operation_type IN ('top_up', 'transfer', 'withdrawal')
    ),
    source_kind TEXT NOT NULL CHECK (
        source_kind IN ('external', 'card', 'savings')
    ),
    source_payment_document_id INTEGER
        REFERENCES payment_documents(id) ON DELETE RESTRICT,
    source_savings_id INTEGER REFERENCES savings(id) ON DELETE RESTRICT,
    destination_kind TEXT NOT NULL CHECK (
        destination_kind IN ('cash', 'card', 'savings')
    ),
    destination_payment_document_id INTEGER
        REFERENCES payment_documents(id) ON DELETE RESTRICT,
    destination_savings_id INTEGER REFERENCES savings(id) ON DELETE RESTRICT,
    amount TEXT NOT NULL CHECK (
        length(amount) > 0
        AND amount NOT GLOB '*[^0-9.]*'
        AND amount NOT LIKE '.%'
        AND amount NOT LIKE '%.'
        AND amount NOT GLOB '*.*.*'
        AND length(replace(replace(amount, '0', ''), '.', '')) > 0
    ),
    currency TEXT NOT NULL CHECK (currency IN ('RUB', 'USD')),
    comment TEXT,
    created_at TEXT NOT NULL,
    CHECK (
        (source_kind = 'external' AND source_payment_document_id IS NULL AND source_savings_id IS NULL)
        OR (source_kind = 'card' AND source_payment_document_id IS NOT NULL AND source_savings_id IS NULL)
        OR (source_kind = 'savings' AND source_payment_document_id IS NULL AND source_savings_id IS NOT NULL)
    ),
    CHECK (
        (destination_kind = 'cash' AND destination_payment_document_id IS NULL AND destination_savings_id IS NULL)
        OR (destination_kind = 'card' AND destination_payment_document_id IS NOT NULL AND destination_savings_id IS NULL)
        OR (destination_kind = 'savings' AND destination_payment_document_id IS NULL AND destination_savings_id IS NOT NULL)
    ),
    CHECK (
        (operation_type = 'top_up' AND source_kind = 'external' AND destination_kind IN ('card', 'savings'))
        OR (
            operation_type = 'transfer'
            AND source_kind IN ('card', 'savings')
            AND destination_kind IN ('card', 'savings')
            AND NOT (
                (source_payment_document_id IS NOT NULL AND source_payment_document_id = destination_payment_document_id)
                OR (source_savings_id IS NOT NULL AND source_savings_id = destination_savings_id)
            )
        )
        OR (operation_type = 'withdrawal' AND source_kind IN ('card', 'savings') AND destination_kind = 'cash')
    )
);

CREATE INDEX transfers_operation_date_idx ON transfers(operation_date);
CREATE INDEX transfers_operation_type_idx ON transfers(operation_type);
CREATE INDEX transfers_source_card_idx ON transfers(source_payment_document_id);
CREATE INDEX transfers_source_savings_idx ON transfers(source_savings_id);
CREATE INDEX transfers_destination_card_idx ON transfers(destination_payment_document_id);
CREATE INDEX transfers_destination_savings_idx ON transfers(destination_savings_id);

CREATE TRIGGER transfers_source_card_valid_insert
BEFORE INSERT ON transfers
WHEN NEW.source_kind = 'card' AND NOT EXISTS (
    SELECT 1 FROM payment_documents AS document
    WHERE document.id = NEW.source_payment_document_id
      AND document.card_type = 'debit'
      AND document.currency IS NOT NULL
      AND document.currency = NEW.currency
)
BEGIN
    SELECT RAISE(ABORT, 'transfer source card must be debit with the same currency');
END;

CREATE TRIGGER transfers_destination_card_valid_insert
BEFORE INSERT ON transfers
WHEN NEW.destination_kind = 'card' AND NOT EXISTS (
    SELECT 1 FROM payment_documents AS document
    WHERE document.id = NEW.destination_payment_document_id
      AND document.card_type = 'debit'
      AND document.currency IS NOT NULL
      AND document.currency = NEW.currency
)
BEGIN
    SELECT RAISE(ABORT, 'transfer destination card must be debit with the same currency');
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

CREATE TRIGGER transfers_preserve_debit_card_type
BEFORE UPDATE OF card_type ON payment_documents
WHEN NEW.card_type <> 'debit' AND EXISTS (
    SELECT 1 FROM transfers
    WHERE source_payment_document_id = OLD.id
       OR destination_payment_document_id = OLD.id
)
BEGIN
    SELECT RAISE(ABORT, 'payment document is used in transfers and must stay debit');
END;

CREATE TRIGGER transfers_preserve_card_currency
BEFORE UPDATE OF currency ON payment_documents
WHEN NEW.currency IS NOT OLD.currency AND EXISTS (
    SELECT 1 FROM transfers
    WHERE source_payment_document_id = OLD.id
       OR destination_payment_document_id = OLD.id
)
BEGIN
    SELECT RAISE(ABORT, 'payment document currency cannot change while it is used in transfers');
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
