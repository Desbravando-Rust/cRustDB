CREATE DATABASE app

CREATE TABLE app.digits (d INT PRIMARY KEY)

INSERT INTO app.digits VALUES (0),(1),(2),(3),(4),(5),(6),(7),(8),(9)

CREATE TABLE app.customers (id INT AUTO_INCREMENT PRIMARY KEY, name VARCHAR(100) NOT NULL COMMENT 'full name', email VARCHAR(100), ci VARCHAR(50) COLLATE utf8mb4_bin, amt DECIMAL(8,2), pic BLOB, UNIQUE KEY email (email)) ENGINE=InnoDB

CREATE TABLE app.orders (id INT AUTO_INCREMENT PRIMARY KEY, customer_id INT NOT NULL, qty INT DEFAULT 1, note TEXT, KEY orders_customer_idx (customer_id, qty) USING BTREE, CONSTRAINT orders_customer_fk FOREIGN KEY (customer_id) REFERENCES app.customers (id) ON UPDATE RESTRICT ON DELETE CASCADE) ENGINE=InnoDB

CREATE VIEW app.order_view AS SELECT id, qty FROM app.orders

CREATE FUNCTION app.add_one(x INT) RETURNS INT DETERMINISTIC RETURN x + 1

CREATE PROCEDURE app.noop() SELECT 1

CREATE TRIGGER app.orders_touch BEFORE INSERT ON app.orders FOR EACH ROW SET NEW.note = 'x'
