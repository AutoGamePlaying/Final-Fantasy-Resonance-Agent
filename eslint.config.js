// Lint rules that keep the package safe and the layers one-way (see CLAUDE.md and
// docs/plan.md §2): every promise awaited, no blocking calls in src/, and no import from a
// lower layer into a higher one.
import js from "@eslint/js";
import { defineConfig } from "eslint/config";
import tseslint from "typescript-eslint";
import layers from "./eslint-rules/layers.js";

const BLOCKING = [
  { name: "node:child_process", importNames: ["execSync", "execFileSync", "spawnSync"] },
  {
    name: "node:fs",
    importNames: ["readFileSync", "writeFileSync", "appendFileSync", "existsSync", "statSync"],
    message: "src/ runs inside dsh: use node:fs/promises.",
  },
];

export default defineConfig(
  { ignores: [".deps/", "build/", "lib/", "node_modules/"] },
  js.configs.recommended,
  tseslint.configs.strictTypeChecked,
  tseslint.configs.stylisticTypeChecked,
  {
    languageOptions: {
      parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname },
    },
    rules: {
      "@typescript-eslint/restrict-template-expressions": ["error", { allowNumber: true }],
      // `a || b` on strings and numbers is the faithful port of Python's `a or b` ("" and 0
      // fall back too); `??` stays required for objects.
      "@typescript-eslint/prefer-nullish-coalescing": [
        "error",
        { ignorePrimitives: { string: true, number: true, boolean: true } },
      ],
      eqeqeq: ["error", "always", { null: "ignore" }],
    },
  },
  {
    files: ["src/**/*.ts"],
    plugins: { local: layers },
    rules: {
      "local/layers": "error",
      "no-restricted-imports": ["error", { paths: BLOCKING }],
    },
  },
  {
    files: ["**/*.js", "**/*.mjs"],
    extends: [tseslint.configs.disableTypeChecked],
    languageOptions: {
      globals: Object.fromEntries(
        ["Buffer", "clearTimeout", "console", "fetch", "process", "setTimeout", "URL"].map(
          (name) => [name, "readonly"],
        ),
      ),
    },
  },
);
