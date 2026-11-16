CREATE TABLE acceptance_records (
    id BIGSERIAL PRIMARY KEY,
    text_value TEXT,
    binary_value BYTEA,
    numeric_value NUMERIC(38, 9),
    integer_value BIGINT,
    float_value DOUBLE PRECISION,
    boolean_value BOOLEAN
);
INSERT INTO acceptance_records
    (text_value, binary_value, numeric_value, integer_value, float_value, boolean_value)
SELECT E'hello 世界 ''quoted'' \\ path\nline', decode('00ff10205c0a', 'hex'),
       12345678901234567890.123456789, -9223372036854775000, 1.25, TRUE
FROM generate_series(1, 12001);
INSERT INTO acceptance_records
    (text_value, binary_value, numeric_value, integer_value, float_value, boolean_value)
VALUES (NULL, NULL, NULL, NULL, NULL, NULL),
       ('', decode('', 'hex'), -0.000000001, 9223372036854775000, -0.5, FALSE);
CREATE INDEX acceptance_records_text ON acceptance_records(text_value);
CREATE VIEW acceptance_summary AS SELECT COUNT(*) AS total FROM acceptance_records;
CREATE SCHEMA acceptance_extra;
CREATE TABLE acceptance_extra.notes (id INTEGER PRIMARY KEY, value TEXT);
INSERT INTO acceptance_extra.notes VALUES (1, 'another schema');
ALTER SEQUENCE acceptance_records_id_seq RESTART WITH 15000;
