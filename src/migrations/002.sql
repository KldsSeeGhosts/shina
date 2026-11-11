CREATE TABLE device_grants (
    device_id TEXT PRIMARY KEY REFERENCES devices(id),
    scopes TEXT NOT NULL
);
INSERT INTO device_grants(device_id,scopes)
    SELECT id,'["read","operate","approve","maintain"]' FROM devices;
PRAGMA user_version=2;
