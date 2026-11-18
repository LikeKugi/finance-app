CREATE TABLE banks (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    comment TEXT NOT NULL DEFAULT ''
);

CREATE TABLE income_categories (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    parent_id INTEGER REFERENCES income_categories(id) ON DELETE RESTRICT,
    is_active INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    CHECK (parent_id IS NULL OR parent_id <> id)
);

CREATE TABLE expense_categories (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    parent_id INTEGER REFERENCES expense_categories(id) ON DELETE RESTRICT,
    is_active INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    CHECK (parent_id IS NULL OR parent_id <> id)
);

CREATE TABLE incomes (
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
    currency TEXT NOT NULL CHECK (currency IN ('RUB', 'USD')),
    comment TEXT NOT NULL DEFAULT ''
);

CREATE TABLE expenses (
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
    currency TEXT NOT NULL CHECK (currency IN ('RUB', 'USD')),
    comment TEXT NOT NULL DEFAULT ''
);

CREATE INDEX incomes_date_id_idx ON incomes(date DESC, id DESC);
CREATE INDEX incomes_category_id_idx ON incomes(category_id);
CREATE INDEX expenses_date_id_idx ON expenses(date DESC, id DESC);
CREATE INDEX expenses_category_id_idx ON expenses(category_id);
CREATE INDEX income_categories_parent_idx ON income_categories(parent_id);
CREATE INDEX expense_categories_parent_idx ON expense_categories(parent_id);