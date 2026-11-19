ALTER TABLE savings ADD COLUMN opened_date TEXT CHECK (
    opened_date IS NULL OR (
        length(opened_date) = 10
        AND opened_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'
        AND date(opened_date, '+0 days') IS NOT NULL
        AND date(opened_date, '+0 days') = opened_date
    )
);

ALTER TABLE savings ADD COLUMN interest_accrual_date TEXT CHECK (
    interest_accrual_date IS NULL OR (
        length(interest_accrual_date) = 10
        AND interest_accrual_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'
        AND date(interest_accrual_date, '+0 days') IS NOT NULL
        AND date(interest_accrual_date, '+0 days') = interest_accrual_date
    )
);