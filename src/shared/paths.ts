/**
 * Where the checkout and its runtime data live. src/shared/ and lib/shared/ are both two levels
 * below the repository root. UE4SS packages, install logs and copied game data stay in .deps/ by
 * default and are never committed; FFR_DEPS moves that directory (for example into /tmp).
 */

import { join, resolve } from "node:path";

export const REPO = resolve(import.meta.dirname, "..", "..");

/** The game-machine settings file, never committed. */
export const MACHINE_ENV = join(REPO, "machine", "machine.env");

/** $FFR_DEPS, else <REPO>/.deps. */
export function depsDir(env: NodeJS.ProcessEnv = process.env): string {
  return env.FFR_DEPS || join(REPO, ".deps");
}

/** <deps>/ue4ss: the dwmapi.dll and ue4ss/ tree to install on the game machine. */
export function ue4ssDir(env: NodeJS.ProcessEnv = process.env): string {
  return join(depsDir(env), "ue4ss");
}

/** <deps>/run: logs and other per-run state produced on the dev box. */
export function runDir(env: NodeJS.ProcessEnv = process.env): string {
  return join(depsDir(env), "run");
}

/** <deps>/r1: the R1 dumps pulled back from the game machine, one directory per job. */
export function r1Dir(env: NodeJS.ProcessEnv = process.env): string {
  return join(depsDir(env), "r1");
}
