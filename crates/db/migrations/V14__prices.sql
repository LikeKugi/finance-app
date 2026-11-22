CREATE TABLE prices (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    date TEXT NOT NULL CHECK (date GLOB '????-??-??'),
    amount TEXT NOT NULL CHECK (
        length(amount) > 0
        AND amount NOT GLOB '*[^0-9.]*'
        AND amount NOT LIKE '.%'
        AND amount NOT LIKE '%.'
        AND amount NOT GLOB '*.*.*'
        AND (instr(amount, '.') = 0 OR length(amount) - instr(amount, '.') <= 12)
        AND length(replace(replace(amount, '0', ''), '.', '')) > 0
    ),
    currency TEXT NOT NULL CHECK (currency IN ('RUB', 'USD')),
    expense_place_id INTEGER NOT NULL REFERENCES expense_places(id) ON DELETE RESTRICT
);

CREATE INDEX prices_date_idx ON prices(date);
CREATE INDEX prices_currency_date_idx ON prices(currency, date);
CREATE INDEX prices_expense_place_date_idx ON prices(expense_place_id, date);