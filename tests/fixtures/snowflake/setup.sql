-- Run only in an isolated test database, using its fixture-owner role.
-- Select the database first. The opt-in live test URL must use schema=TEST_SCHEMA.
CREATE SCHEMA TEST_SCHEMA;
CREATE TABLE TEST_SCHEMA."Type Samples" (
    ID NUMBER(38,0) NOT NULL COMMENT 'synthetic identifier',
    "Exact Decimal" NUMBER(38,18),
    "MixedCase" VARCHAR DEFAULT 'default-value',
    FLAG BOOLEAN,
    D DATE,
    T TIME(9),
    NTZ TIMESTAMP_NTZ(9),
    LTZ TIMESTAMP_LTZ(9),
    TZ TIMESTAMP_TZ(9),
    BIN BINARY,
    PAYLOAD VARIANT
);
INSERT INTO TEST_SCHEMA."Type Samples"
SELECT SEQ4(),
    12345678901234567890.123456789012345678::NUMBER(38,18),
    IFF(MOD(SEQ4(),7)=0,NULL,'synthetic-' || SEQ4()),
    MOD(SEQ4(),2)=0,
    '2026-11-01'::DATE,
    '12:34:56.123456789'::TIME,
    '2026-11-01 12:34:56.123456789'::TIMESTAMP_NTZ,
    '2026-11-01 12:34:56.123456789 +05:30'::TIMESTAMP_LTZ,
    '2026-11-01 12:34:56.123456789 +05:30'::TIMESTAMP_TZ,
    TO_BINARY('00FF10','HEX'),
    PARSE_JSON('{"kind":"synthetic","nested":{"value":42}}')
FROM TABLE(GENERATOR(ROWCOUNT => 125));
CREATE VIEW TEST_SCHEMA."Sample View" AS
SELECT ID, "Exact Decimal", "MixedCase", PAYLOAD
FROM TEST_SCHEMA."Type Samples" WHERE ID < 20;
CREATE SCHEMA "CaseSchema";
CREATE TABLE "CaseSchema"."Quoted Table" ("CaseColumn" VARCHAR);
INSERT INTO "CaseSchema"."Quoted Table" VALUES ('synthetic quoted namespace');
-- Grant the test role USAGE on the database, both schemas and the test warehouse,
-- plus SELECT on these tables and view. Do not grant writes to the test role.
