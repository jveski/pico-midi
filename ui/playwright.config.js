// @ts-check
const { defineConfig } = require("@playwright/test");
const path = require("path");

module.exports = defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  retries: 0,
  workers: 1,
  reporter: "list",
  timeout: 15000,
  use: {
    baseURL: `file://${path.resolve(__dirname, "index.html")}`,
    // No real browser needed - we load the file directly
    trace: "off",
  },
  projects: [
    {
      name: "chromium",
      use: {
        browserName: "chromium",
      },
    },
  ],
});
