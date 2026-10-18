CREATE SCHEMA app;
CREATE TYPE app.mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE app.pair AS (a int, b text);
CREATE DOMAIN app.posint AS int CHECK (VALUE > 0);
CREATE TABLE app.customers (
    id serial PRIMARY KEY,
    name text NOT NULL,
    email text UNIQUE,
    mood app.mood
);
COMMENT ON COLUMN app.customers.name IS 'full name';
CREATE TABLE app.orders (
    id serial PRIMARY KEY,
    customer_id int NOT NULL REFERENCES app.customers (id) ON UPDATE RESTRICT ON DELETE CASCADE,
    qty app.posint DEFAULT 1,
    note text
);
CREATE INDEX orders_customer_idx ON app.orders (customer_id, qty);
CREATE VIEW app.order_view AS SELECT id, qty FROM app.orders;
CREATE MATERIALIZED VIEW app.order_mv AS SELECT customer_id, count(*) AS n FROM app.orders GROUP BY 1;
CREATE FUNCTION app.add_one(x int) RETURNS int LANGUAGE sql AS 'SELECT x + 1';
CREATE FUNCTION app.add_one(x int, y int) RETURNS int LANGUAGE sql AS 'SELECT x + y + 1';
CREATE PROCEDURE app.noop() LANGUAGE sql AS 'SELECT 1';
CREATE FUNCTION app.touch() RETURNS trigger LANGUAGE plpgsql AS 'BEGIN RETURN NEW; END';
CREATE TRIGGER orders_touch BEFORE INSERT ON app.orders FOR EACH ROW EXECUTE FUNCTION app.touch();
CREATE SEQUENCE app.ticket_seq;
CREATE TABLE public.events (id int, day date) PARTITION BY RANGE (day);
CREATE TABLE public.events_2024 PARTITION OF public.events FOR VALUES FROM ('2024-01-01') TO ('2025-01-01');
