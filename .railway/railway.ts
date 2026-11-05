import { defineRailway, github, preserve, project, service } from "railway/iac";

// Infrastructure for the RemakeBackend API. The project name comes from the
// linked Railway environment. The GitHub repository and branch come from the
// environment (RAILWAY_SOURCE_REPO, RAILWAY_SOURCE_BRANCH) so that no account
// name is committed. Variables are preserve()d: only their names appear here.
export default defineRailway((ctx) => {
  const repo = process.env.RAILWAY_SOURCE_REPO;
  if (!repo) {
    throw new Error("Set RAILWAY_SOURCE_REPO to the owner/name of the GitHub repository connected to this service");
  }
  const api = service("RemakeBackend", {
    source: github(repo, { branch: process.env.RAILWAY_SOURCE_BRANCH ?? "master" }),
    env: {
      DATABASE_URL: preserve(),
      AUTH_SECRET: preserve(),
      APP_ORIGIN: preserve(),
      PORT: preserve(),
    },
    // Server start. Never applies migrations.
    start: "bash /usr/local/bin/start-cloud",
    // One-shot migration before the new container starts. A failure stops the deployment.
    preDeploy: "cloud-api migrate",
    // A deployment is promoted only after this returns 200.
    healthcheck: "/health/ready",
    healthcheckTimeout: 120,
  });

  return project(ctx.projectName, { resources: [api] });
});
