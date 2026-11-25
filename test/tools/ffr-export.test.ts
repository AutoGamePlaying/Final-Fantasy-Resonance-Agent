import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import { extname, join } from "node:path";
import test from "node:test";

const TOOL_DIRECTORY = join(import.meta.dirname, "..", "..", "tools", "ffr-export");

async function readSources(): Promise<Map<string, string>> {
  const names = await readdir(TOOL_DIRECTORY);
  const sourceNames = names.filter((name) => [".cs", ".csproj", ".sh"].includes(extname(name)));
  assert.ok(sourceNames.length >= 3);
  return new Map(
    await Promise.all(
      sourceNames.map(async (name): Promise<readonly [string, string]> => [
        name,
        await readFile(join(TOOL_DIRECTORY, name), "utf8"),
      ]),
    ),
  );
}

void test("ffr-export never initializes proprietary Oodle or uses web clients", async () => {
  const sources = await readSources();
  for (const [name, source] of sources) {
    for (const forbidden of ["OodleHelper", "DownloadOodle", "HttpClient", "WebClient"]) {
      assert.ok(!source.includes(forbidden), `${name} contains ${forbidden}`);
    }
  }
});

void test("ffr-export sources contain no machine-specific absolute paths", async () => {
  const sources = await readSources();
  for (const [name, source] of sources) {
    assert.doesNotMatch(source, /\/root\//u, name);
    assert.doesNotMatch(source, /[A-Za-z]:[\\/]/u, name);
  }
});

void test("ffr-export pins the required CUE4Parse version", async () => {
  const project = await readFile(join(TOOL_DIRECTORY, "FfrExport.csproj"), "utf8");
  assert.match(project, /<PackageReference Include="CUE4Parse" Version="1\.2\.2\.202611" \/>/u);
});

void test("ffr-export checks its manifest before clearing a directory", async () => {
  const program = await readFile(join(TOOL_DIRECTORY, "Program.cs"), "utf8");
  assert.match(program, /RequireExportManifest\(outputPath, outputArgument\)/u);
  assert.match(program, /Path\.Combine\(outputPath, "manifest\.json"\)/u);
  assert.match(program, /manifest\.Property\("cue4ParseVersion"\) is null/u);
});

void test("ffr-export locres uses CUE4Parse FTextLocalizationResource and guards output directory", async () => {
  const program = await readFile(join(TOOL_DIRECTORY, "Program.cs"), "utf8");
  assert.match(program, /case "locres":/u);
  assert.match(program, /new FTextLocalizationResource\(/u);
  assert.match(program, /DefaultLocalizationDirectory = "\.deps\/ffr\/export\/Localization"/u);
  assert.match(program, /DefaultLanguages = \["zh-Hans", "en", "ja"\]/u);
  assert.match(program, /PrepareOutputDirectory\(outputPath, outputArgument, force, "locres"\)/u);
  assert.match(program, /EnsureManifestMatchesCommand\(/u);
});
