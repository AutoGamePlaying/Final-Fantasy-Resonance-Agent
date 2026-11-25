/**
 * Paths inside the unpacked R2 export tree (<deps>/ffr/export/). The export directory holds
 * game-derived JSON produced by tools/ffr-export and is never committed to git.
 */

import { join } from "node:path";
import { depsDir } from "../shared/index.ts";

const GAME_MOUNT_PREFIX = "/Game/";

/** <deps>/ffr/export: root directory produced by tools/ffr-export export. */
export function exportDir(env: NodeJS.ProcessEnv = process.env): string {
  return join(depsDir(env), "ffr", "export");
}

/** <exportRoot>/FFRS/Content/Datatable: exported DataTable, StringTable and DataAsset JSONs. */
export function datatableDir(root: string = exportDir()): string {
  return join(root, "FFRS", "Content", "Datatable");
}

/** <exportRoot>/Localization: per-language JSON files produced by tools/ffr-export locres. */
export function localizationDir(root: string = exportDir()): string {
  return join(root, "Localization");
}

/** <exportRoot>/Localization/<lang>.json: exported Game.locres JSON for one language. */
export function localizationFile(language: string, root: string = exportDir()): string {
  return join(localizationDir(root), `${language}.json`);
}

/**
 * Map a UE object or package path (for example
 * "/Game/Datatable/Strings/ChapterTitle/ST_ChapterTitle.ST_ChapterTitle" or
 * "/Game/Datatable/Scenario/DT_ChapterData") to its JSON path under the export root.
 * Returns null if the path does not sit cleanly inside /Game/.
 */
export function ueObjectPathToExportJson(uePath: string, root: string): string | null {
  if (!uePath.startsWith(GAME_MOUNT_PREFIX)) return null;
  const withoutMount = uePath.slice(GAME_MOUNT_PREFIX.length);
  if (withoutMount === "") return null;
  const segments = withoutMount.split("/");
  const last = segments.at(-1);
  if (last === undefined || last === "") return null;
  const dot = last.indexOf(".");
  const assetBase = dot >= 0 ? last.slice(0, dot) : last;
  if (assetBase === "") return null;
  const normalizedSegments = [...segments.slice(0, -1), assetBase];
  for (const segment of normalizedSegments) {
    if (segment === "" || segment === "." || segment === ".." || segment.includes("\\")) {
      return null;
    }
  }
  return `${join(root, "FFRS", "Content", ...normalizedSegments)}.json`;
}
