ALTER TABLE payment_documents
ADD COLUMN name TEXT CHECK (name IS NULL OR length(trim(name)) > 0);

CREATE TABLE expense_places (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    comment TEXT
);

ALTER TABLE expenses
ADD COLUMN expense_place_id INTEGER REFERENCES expense_places(id) ON DELETE RESTRICT;

CREATE INDEX expenses_expense_place_id_idx ON expenses(expense_place_id);