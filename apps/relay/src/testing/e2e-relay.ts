// Test support, excluded from the build: a real relay for end-to-end tests of the desktop app's
// relay transport (`relay_e2e.rs` in `apps/desktop/src-tauri`). It uses the stand-in Clerk
// instance, so no credentials or network access are needed.
//
//   node src/testing/e2e-relay.ts <transcript file> <log file>
//
// Prints one JSON line, `{ "url", "tokens": { "user_A", "user_B" } }`, then serves until killed.
// Everything devices send in binary frames is appended to the transcript, so the test can check
// that the relay only ever received ciphertext. Logs go to the log file at `debug` level, so the
// test can check that they contain no clipboard content either.

import { appendFileSync, createWriteStream, writeFileSync } from "node:fs";
import type { WebSocket } from "@fastify/websocket";
import { buildServer } from "../server.ts";
import { accessToken, testConfig } from "./clerk.ts";

const [transcript, logFile] = process.argv.slice(2);
if (!transcript || !logFile) {
  console.error("usage: node src/testing/e2e-relay.ts <transcript file> <log file>");
  process.exit(2);
}
writeFileSync(transcript, "");

const app = await buildServer(testConfig({ LOG_LEVEL: "debug" }), {
  logStream: createWriteStream(logFile),
});
app.websocketServer.on("connection", (socket: WebSocket) => {
  socket.on("message", (data: Buffer, isBinary: boolean) => {
    if (isBinary) appendFileSync(transcript, data);
  });
});
await app.listen({ host: "127.0.0.1", port: 0 });

const address = app.server.address();
if (!address || typeof address === "string") throw new Error("no TCP address");
console.log(
  JSON.stringify({
    url: `http://127.0.0.1:${address.port}`,
    // Valid for 10 minutes.
    tokens: { user_A: accessToken("user_A"), user_B: accessToken("user_B") },
  }),
);
