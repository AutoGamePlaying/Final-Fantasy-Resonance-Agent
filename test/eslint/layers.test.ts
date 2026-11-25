import { describe, it } from "node:test";
import { RuleTester } from "eslint";
import { rule } from "../../eslint-rules/layers.js";

const ruleTester = new RuleTester({
  languageOptions: { ecmaVersion: "latest", sourceType: "module" },
});

RuleTester.describe = describe;
RuleTester.it = it;

ruleTester.run("layers", rule, {
  valid: [
    {
      name: "data may import shared through its index",
      filename: "src/data/uses-shared.ts",
      code: 'import { text } from "../shared/index.ts";',
    },
    {
      name: "same-layer imports may go deep",
      filename: "src/skills/uses-neighbour.ts",
      code: 'import { move } from "./move.ts";',
    },
    {
      name: "skills may import io/types.ts",
      filename: "src/skills/uses-io-types.ts",
      code: 'import "../io/types.ts";',
    },
    {
      name: "io/live may import shared through its index",
      filename: "src/io/live/uses-shared.ts",
      code: 'import { records } from "../../shared/index.ts";',
    },
    {
      name: "io/live may import another io/live file",
      filename: "src/io/live/uses-neighbour.ts",
      code: 'import { connect } from "./bridge.ts";',
    },
    {
      name: "host may import a lower layer through its index",
      filename: "src/host/uses-data.ts",
      code: 'import { query } from "../data/index.ts";',
    },
    {
      name: "host may deep-import the io implementations it injects",
      filename: "src/host/assembles-io.ts",
      code: 'import { connect } from "../io/live/index.ts";\nimport { simulate } from "../io/sim/index.ts";',
    },
  ],
  invalid: [
    {
      name: "data cannot import skills",
      filename: "src/data/uses-skills.ts",
      code: 'import { plan } from "../skills/index.ts";',
      errors: [{ messageId: "layerOrder", data: { from: "data", to: "skills" } }],
    },
    {
      name: "skills cannot import io/live",
      filename: "src/skills/uses-live.ts",
      code: 'import { connect } from "../io/live/machine.ts";',
      errors: [
        {
          messageId: "ioEntry",
          data: { from: "skills", target: "io/live/machine.ts" },
        },
      ],
    },
    {
      name: "io/live cannot import io/sim",
      filename: "src/io/live/uses-sim.ts",
      code: 'import { simulate } from "../sim/index.ts";',
      errors: [{ messageId: "liveSim", data: { from: "io/live", to: "io/sim" } }],
    },
    {
      name: "host cannot deep-import shared",
      filename: "src/host/uses-shared-deep.ts",
      code: 'import { text } from "../shared/values.ts";',
      errors: [
        {
          messageId: "crossLayer",
          data: { to: "shared", target: "shared/values.ts" },
        },
      ],
    },
  ],
});
