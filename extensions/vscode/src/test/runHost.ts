// Launches VS Code with the extension under development and runs host tests.
// Usage: npm run test:host  (set KARA_BIN to a built binary so the agent can start)
import * as path from "path";
import { runTests } from "@vscode/test-electron";

async function main(): Promise<void> {
  const root = path.resolve(__dirname, "../..");
  await runTests({
    extensionDevelopmentPath: root,
    extensionTestsPath: path.join(root, "out/test/host/index.js"),
    launchArgs: [path.resolve(root, "../../tests/fixtures/repos/python-shop"), "--disable-extensions"],
    extensionTestsEnv: process.env.KARA_BIN ? { PATH: `${path.dirname(process.env.KARA_BIN)}${path.delimiter}${process.env.PATH}` } : undefined,
  });
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
