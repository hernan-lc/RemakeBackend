-- Historical integrity_passed rows are deliberately retained: they were never scanned.
ALTER TABLE cloud_verification_jobs DROP CONSTRAINT cloud_verification_job_status;
ALTER TABLE cloud_verification_jobs ADD CONSTRAINT cloud_verification_job_status CHECK (status IN ('queued','running','integrity_passed','antivirus_passed','rejected','failed'));
ALTER TABLE cloud_verification_jobs DROP CONSTRAINT cloud_verification_jobs_result_class_check;
ALTER TABLE cloud_verification_jobs ADD CONSTRAINT cloud_verification_jobs_result_class_check CHECK (result_class IN ('integrity_passed','antivirus_passed','invalid_package','key_not_authorized','antivirus_alert','temporary_failure'));
ALTER TABLE cloud_plugin_releases DROP CONSTRAINT cloud_plugin_release_status;
ALTER TABLE cloud_plugin_releases ADD CONSTRAINT cloud_plugin_release_status CHECK (status IN ('awaiting_verification','integrity_passed','antivirus_passed','rejected'));
