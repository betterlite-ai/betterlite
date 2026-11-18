A private Redis server for one test, started from Debian's redis-server binary.

`RedisFixture::start` launches redis-server on a unix socket in a fresh temp folder, with TCP off and snapshots and the append-only file disabled, and returns once the socket answers PING. The fixture hands out only its `redis+unix://` URL, so a test cannot reach a Redis that runs anywhere else. Dropping the fixture kills the server, waits for it and removes the folder. A missing binary is a typed error that names redis-server and the Debian package.

Every test that needs Redis starts its own fixture. The Linux tests read `/proc` to prove the server process is gone after the drop; other platforms check only the folder.
