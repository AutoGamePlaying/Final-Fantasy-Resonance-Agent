#!/usr/bin/env node
// Stand-in for ssh/scp in tests: records argv and stdin, then replays scripted stdout/stderr/
// exit code from FAKE_SSH_CONFIG. It never runs on a real game machine.

import { appendFileSync, readFileSync, statSync, writeFileSync } from "node:fs";

const configPath = process.env.FAKE_SSH_CONFIG;
if (!configPath) {
  process.stderr.write("FAKE_SSH_CONFIG is not set\n");
  process.exit(90);
}
const config = JSON.parse(readFileSync(configPath, "utf8"));
const counterPath = `${configPath}.count`;
const count = readCount();
writeFileSync(counterPath, String(count + 1));

const args = process.argv.slice(2);
const logPath = process.env.FAKE_SSH_LOG;
if (logPath) {
  const entry = { args, stdin: readStdin() };
  if (config.captureUploaded === true) {
    const uploadedBase64 = captureUploadedFile(args);
    if (uploadedBase64 !== undefined) entry.uploadedBase64 = uploadedBase64;
  }
  appendFileSync(logPath, `${JSON.stringify(entry)}\n`, "utf8");
}

const responses = Array.isArray(config.responses) ? config.responses : [];
const response = responses[count] ?? config.default ?? {};
if (typeof response.delayMs === "number" && response.delayMs > 0) {
  await new Promise((resolve) => setTimeout(resolve, response.delayMs));
}
if (typeof response.stdoutBase64 === "string") {
  process.stdout.write(Buffer.from(response.stdoutBase64, "base64"));
} else if (response.stdout) {
  process.stdout.write(response.stdout);
}
if (typeof response.stderrBase64 === "string") {
  process.stderr.write(Buffer.from(response.stderrBase64, "base64"));
} else if (response.stderr) {
  process.stderr.write(response.stderr);
}
process.exit(typeof response.code === "number" ? response.code : 0);

function readCount() {
  try {
    return Number(readFileSync(counterPath, "utf8"));
  } catch {
    return 0;
  }
}

function readStdin() {
  try {
    return readFileSync(0, "utf8");
  } catch {
    return "";
  }
}

function captureUploadedFile(argv) {
  if (argv.length < 2) return undefined;
  const source = argv[argv.length - 2];
  const destination = argv[argv.length - 1];
  if (!source || !destination || !destination.includes(":")) return undefined;
  try {
    if (!statSync(source).isFile()) return undefined;
    return readFileSync(source).toString("base64");
  } catch {
    return undefined;
  }
}
