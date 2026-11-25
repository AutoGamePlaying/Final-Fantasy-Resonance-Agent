/**
 * Shared plumbing for the command lines: one Chinese error handler, the dry-run prefix, and the
 * "the game must be closed" check used by every command that writes to the game directory.
 */

import {
  CalledProcessError,
  isGameRunning,
  loadMachine,
  type Machine,
} from "../../io/live/machine.ts";
import { describeError } from "../../shared/index.ts";

export async function loadConfiguredMachine(path: string | undefined): Promise<Machine> {
  return loadMachine(path);
}

/** Refuse to touch Win64 while any exe in it is running. The log reader is exempt. */
export async function assertGameStopped(machine: Machine): Promise<void> {
  const running = await isGameRunning(machine);
  if (running.length > 0) {
    throw new Error(`游戏正在运行(${running.join("、")}),请先退出游戏再操作`);
  }
}

export function requireAppId(machine: Machine): string {
  const appId = machine.appId?.trim();
  if (!appId) {
    throw new Error("machine.env 里缺少 FFR_APP_ID:填上 Steam AppID 才能启动游戏");
  }
  if (!/^\d+$/u.test(appId)) {
    throw new Error(`FFR_APP_ID 必须是数字,收到 ${appId}`);
  }
  return appId;
}

export function printDryRun(lines: readonly string[]): void {
  for (const line of lines) console.log(`[dry-run] ${line}`);
}

/** Run one command line, print any failure in Chinese, and set the process exit code. */
export async function runCli(main: () => Promise<number>): Promise<void> {
  try {
    process.exitCode = await main();
  } catch (error) {
    const code = error instanceof CalledProcessError ? error.returnCode || 1 : 1;
    console.error(`错误:${describeError(error)}`);
    process.exitCode = code;
  }
}
