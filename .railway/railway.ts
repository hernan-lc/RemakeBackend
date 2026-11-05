import { defineRailway, github, preserve, project, service } from "railway/iac";

// Railway infrastructure for RemakeBackend (project robust-charisma, production).
// Variables are preserve()d: their values stay in the Railway dashboard and are
// never committed here. Only the names are declared.
export default defineRailway(() => {
  const api = service("RemakeBackend", {
    source: github("hernan-lc/RemakeBackend", { branch: "master" }),
    env: {
      DATABASE_URL: preserve(),
      AUTH_SECRET: preserve(),
      APP_ORIGIN: preserve(),
      PORT: preserve(),
    },
    // The image is built from deploy/Dockerfile.
    // Normal server start: never applies migrations.
    start: "bash /usr/local/bin/start-cloud",
    // One-shot migration before the new container starts. A failure stops the deployment.
    preDeploy: "cloud-api migrate",
    // A deployment is promoted only after this returns 200.
    healthcheck: "/health/ready",
    healthcheckTimeout: 120,
    replicas: { sfo: 1 },
  });

  return project("robust-charisma", { resources: [api] });
});
