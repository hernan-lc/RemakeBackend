-- Display metadata for verified plugin releases. Filled only when a release
-- passes verification, from the signed manifest. Presentation fields are
-- never authorization inputs; installation still uses the signed package.
ALTER TABLE cloud_plugin_releases
    ADD COLUMN display_name TEXT CHECK (display_name IS NULL OR length(display_name) BETWEEN 1 AND 120),
    ADD COLUMN summary TEXT CHECK (summary IS NULL OR length(summary) BETWEEN 1 AND 500),
    ADD COLUMN categories TEXT[] NOT NULL DEFAULT '{}' CHECK (cardinality(categories) <= 8),
    ADD COLUMN authors TEXT[] NOT NULL DEFAULT '{}' CHECK (cardinality(authors) <= 8),
    ADD COLUMN engine TEXT CHECK (engine IS NULL OR length(engine) BETWEEN 1 AND 64),
    ADD COLUMN homepage TEXT CHECK (homepage IS NULL OR length(homepage) BETWEEN 1 AND 200),
    ADD COLUMN dependencies JSONB NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(dependencies) = 'object');
