import reactHooks from "eslint-plugin-react-hooks";
import { defineConfig, globalIgnores } from "eslint/config";
import tseslint from "typescript-eslint";

export default defineConfig(globalIgnores(["dist", "public", "eslint.config.js"]), tseslint.configs.strictTypeChecked, tseslint.configs.stylisticTypeChecked, reactHooks.configs.flat.recommended, {
  languageOptions: { parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname } },
  rules: {
    "@typescript-eslint/restrict-template-expressions": ["error", { allowNumber: true }],
    // uPlot declares its option enums as const enums, which isolatedModules cannot read, and erasableSyntaxOnly keeps enums out of this code.
    "@typescript-eslint/no-unsafe-enum-assignment": "off",
  },
});
