import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  // `npm run dev` proxies API calls to a running `preflight serve`.
  server: { proxy: { "/api": "http://localhost:7777" } },
});
