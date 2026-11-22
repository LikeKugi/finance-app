-- Добавляет поле «Размер следующего платежа» для платёжных карт.
-- Поле опционально: NULL для дебетовых карт и для случаев, когда сумма неизвестна.
ALTER TABLE payment_documents ADD COLUMN next_payment_amount TEXT CHECK (
    next_payment_amount IS NULL OR (
        length(next_payment_amount) > 0
        AND next_payment_amount NOT GLOB '*[^0-9.]*'
        AND next_payment_amount NOT LIKE '.%'
        AND next_payment_amount NOT LIKE '%.'
        AND next_payment_amount NOT GLOB '*.*.*'
        AND length(replace(replace(next_payment_amount, '0', ''), '.', '')) > 0
    )
);
