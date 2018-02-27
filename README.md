# setup cockroachdb

```sh
docker volume create --name cockroach-c472921b-db
docker run --name cockroach-c472921b --hostname=roach-c472921b -v cockroach-c472921b-db:/cockroach/cockroach-data -p 26257:26257 -p 8080:8080 --memory="512m" --memory-swap="512m" --cpuset-cpus="2,3" -d cockroachdb/cockroach-unstable:v2.0-alpha.20180212 start --cache="25%" --max-sql-memory="25%" --insecure
```

```sh
docker exec -it cockroach-c472921b ./cockroach user set testuser --insecure
docker exec -it cockroach-c472921b ./cockroach sql --insecure
```

```sql
CREATE DATABASE testdb;

GRANT ALL ON DATABASE testdb TO testuser;

USE testdb;

CREATE TABLE docs (
  user_id uuid,
  doc_id uuid,
  revision int,
  payload bytes,
  PRIMARY KEY (user_id ASC, doc_id ASC, revision DESC)
);
```

# setup postgresql

```sh
docker volume create --name postgres-c472921b-db
docker run --name postgres-c472921b -v postgres-c472921b-db:/var/lib/postgresql/data -p 5432:5432 --memory="512m" --memory-swap="512m" --cpuset-cpus="2,3" -e POSTGRES_PASSWORD=testpw -d postgres:10.2-alpine
```

```sh
docker run -it --rm --link postgres-c472921b:postgres postgres psql -h postgres -U postgres
```

```sql
CREATE DATABASE testdb;

\connect testdb;

CREATE TABLE docs (
  user_id uuid,
  doc_id uuid,
  revision bigint,
  payload bytea,
  PRIMARY KEY (user_id, doc_id, revision)
);
```

# run tests

```sh
cargo run -- load cockroachdb
cargo run -- run cockroachdb 0
cargo run -- run cockroachdb 1 # NOTE: very slow, must be aborted
```

```sh
cargo run -- load postgresql
cargo run -- run postgresql 0
cargo run -- run postgresql 1
cargo run -- run postgresql 2
cargo run -- run postgresql 4
```

# cleanup

```sh
docker stop cockroach-c472921b
docker stop postgres-c472921b
docker rm cockroach-c472921b
docker rm postgres-c472921b
docker volume rm cockroach-c472921b-db
docker volume rm postgres-c472921b-db
```
