ALTER TABLE savings
    ADD COLUMN currency TEXT REFERENCES currencies(code) ON DELETE RESTRICT;

CREATE INDEX savings_currency_idx ON savings(currency);