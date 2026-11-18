CREATE TABLE savings (
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
    interest_destination TEXT NOT NULL CHECK (interest_destination IN ('reinvest', 'withdraw')),
    is_active INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    CHECK ((rate_after_threshold IS NULL) = (threshold_amount IS NULL)),
    CHECK (
        (term_type = 'limited' AND term_end_date IS NOT NULL AND length(term_end_date) = 10)
        OR (term_type = 'unlimited' AND term_end_date IS NULL)
    )
);

CREATE INDEX savings_bank_active_title_idx
    ON savings(bank_id, is_active, title COLLATE NOCASE);