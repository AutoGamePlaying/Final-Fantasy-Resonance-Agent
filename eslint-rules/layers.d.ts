import type { Rule } from "eslint";

export declare const rule: Rule.RuleModule;

declare const plugin: {
  meta: { name: string; version: string };
  rules: { layers: Rule.RuleModule };
};
export default plugin;
