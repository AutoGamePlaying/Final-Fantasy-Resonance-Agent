// Layering rules for src/. Imports are resolved to file paths before they are compared, so a
// violation cannot hide behind another spelling of the same path. The layer order comes from
// docs/plan.md §2: shared ← data ← io ← skills ← runner ← planner ← host.
import { dirname, extname, join, resolve, sep } from "node:path";

const LAYER_ORDER = new Map([
  ["shared", 0],
  ["data", 1],
  ["io", 2],
  ["skills", 3],
  ["runner", 4],
  ["planner", 5],
  ["host", 6],
]);

function srcPath(file) {
  const parts = resolve(file).split(sep);
  const index = parts.findIndex(
    (part, position) => part === "src" && LAYER_ORDER.has(parts[position + 1] ?? ""),
  );
  if (index < 0) return undefined;
  const layer = parts[index + 1];
  return { layer, path: parts.slice(index + 1).join("/") };
}

function resolveImport(importer, specifier) {
  const base = resolve(dirname(importer), specifier);
  return extname(base) === "" ? join(base, "index.ts") : base;
}

function isIoImplementation(path) {
  return path.startsWith("io/live/") || path.startsWith("io/sim/");
}

export const rule = {
  meta: {
    type: "problem",
    docs: {
      description: "Enforce the src/ layer order and the import entry points on both sides.",
    },
    schema: [],
    messages: {
      layerOrder: "分层规则 1：{{from}} 层不能 import 更高层的 {{to}} 层；依赖只能从高层指向低层。",
      ioEntry:
        "分层规则 2：{{from}} 层只能 import src/io/index.ts 或 src/io/types.ts，不能 import {{target}}。",
      liveSim: "分层规则 3：src/io/live 和 src/io/sim 不能互相 import（{{from}} → {{to}}）。",
      crossLayer:
        "分层规则 4：跨层 import 只能指向 {{to}}/index.ts（例外：io/types.ts），不能 import {{target}}。",
    },
  },

  create(context) {
    const importer = srcPath(context.filename);
    if (!importer) return {};

    function check(node) {
      if (node.source?.type !== "Literal" || typeof node.source.value !== "string") return;
      const specifier = node.source.value;
      if (!specifier.startsWith(".")) return;

      const target = srcPath(resolveImport(context.filename, specifier));
      if (!target) return;

      const from = importer.layer;
      const to = target.layer;
      const data = { from, to, target: target.path };

      if (LAYER_ORDER.get(from) < LAYER_ORDER.get(to)) {
        context.report({ node, messageId: "layerOrder", data });
        return;
      }

      if (from !== "io" && from !== "host" && to === "io" && !isIoEntry(target.path)) {
        context.report({ node, messageId: "ioEntry", data });
        return;
      }

      if (from === "io" && to === "io" && areLiveAndSim(importer.path, target.path)) {
        const side = importer.path.startsWith("io/live/") ? "io/live" : "io/sim";
        context.report({
          node,
          messageId: "liveSim",
          data: { from: side, to: side === "io/live" ? "io/sim" : "io/live" },
        });
        return;
      }

      if (from === to) return;

      // The host is the composition root: rule 2 deliberately lets it deep-import io/live and
      // io/sim, so the entry-point rule below must not undo that exemption.
      const hostIoImplementation =
        from === "host" && to === "io" && isIoImplementation(target.path);
      if (!isLayerEntry(to, target.path) && !hostIoImplementation) {
        context.report({ node, messageId: "crossLayer", data });
      }
    }

    return {
      ImportDeclaration: check,
      ImportExpression: check,
      ExportNamedDeclaration: check,
      ExportAllDeclaration: check,
    };
  },
};

function isIoEntry(path) {
  return path === "io/index.ts" || path === "io/types.ts";
}

function isLayerEntry(layer, path) {
  return path === `${layer}/index.ts` || path === "io/types.ts";
}

function areLiveAndSim(importer, target) {
  return (
    (importer.startsWith("io/live/") && target.startsWith("io/sim/")) ||
    (importer.startsWith("io/sim/") && target.startsWith("io/live/"))
  );
}

export default {
  meta: { name: "local-layers", version: "0.0.0" },
  rules: { layers: rule },
};
