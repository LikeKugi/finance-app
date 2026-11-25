ALTER TABLE expense_categories
ADD COLUMN is_regular INTEGER NOT NULL DEFAULT 0 CHECK (is_regular IN (0, 1));
