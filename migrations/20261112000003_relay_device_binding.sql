-- A live fencing lease identifies the enrolled device that owns the socket.
ALTER TABLE cloud_relay_leases ADD COLUMN device_id UUID REFERENCES cloud_remote_devices(id);
