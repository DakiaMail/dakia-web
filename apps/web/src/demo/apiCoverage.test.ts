// @vitest-environment node
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { expect, it } from "vitest";

// A new request declaration must be classified here and implemented (or
// deliberately denied) by the sandbox. This catches drift outside smoke paths.
const root = fileURLToPath(new URL("../", import.meta.url));
function normalize(node: ts.Expression): string {
  if (ts.isStringLiteralLike(node)) return node.text;
  if (ts.isCallExpression(node) && node.expression.getText() === "accountPath") return "/accounts/:id";
  if (ts.isTemplateExpression(node)) return node.head.text + node.templateSpans.map(span => {
    const expression = span.expression;
    const value = ts.isCallExpression(expression) && ts.isIdentifier(expression.expression)
      ? expression.expression.text === "accountPath" ? "/accounts/:id" : expression.expression.text === "messagePath" ? "/accounts/:id/messages/:id" : ":id"
      : ":id";
    return value + span.literal.text;
  }).join("").replace("/conversation:id", "/conversation");
  return `dynamic:${node.getText()}`;
}

it("requires an explicit sandbox decision whenever the app adds an API surface", () => {
  const routes = new Set<string>();
  for (const filename of readdirSync(root, { recursive: true }) as string[]) {
    if (!/\.tsx?$/.test(filename) || filename.includes(".test.") || filename.startsWith("demo/")) continue;
    const source = ts.createSourceFile(filename, readFileSync(root + filename, "utf8"), ts.ScriptTarget.Latest, true);
    const ownsApiRequest = filename === "api.ts" || source.statements.some(statement =>
      ts.isImportDeclaration(statement) && /(?:^|\/)api$/.test((statement.moduleSpecifier as ts.StringLiteral).text)
      && statement.importClause?.namedBindings && ts.isNamedImports(statement.importClause.namedBindings)
      && statement.importClause.namedBindings.elements.some(element => element.name.text === "request"));
    if (!ownsApiRequest) continue;
    const visit = (node: ts.Node) => {
      if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === "request" && node.arguments[0]) routes.add(normalize(node.arguments[0]).split("?")[0]);
      ts.forEachChild(node, visit);
    };
    visit(source);
  }
  expect([...routes].sort()).toEqual([
    "/accounts", "/accounts/:id", "/accounts/:id/attachments", "/accounts/:id/drafts", "/accounts/:id/drafts/:id",
    "/accounts/:id/folders", "/accounts/:id/jobs/:id", "/accounts/:id/messages", "/accounts/:id/messages/:id",
    "/accounts/:id/messages/:id/conversation", "/accounts/:id/messages/:id/image", "/accounts/:id/messages/:id/sender-mail",
    "/accounts/:id/messages/:id/translation", "/accounts/:id/operations", "/accounts/:id/operations/:id", "/accounts/:id/probe",
    "/accounts/:id/recipients", "/accounts/:id/rule-activity", "/accounts/:id/scheduled", "/accounts/:id/sender-cleanups",
    "/accounts/:id/settings", "/accounts/:id/submissions", "/accounts/:id/submissions/:id", "/accounts/:id/submissions/:id/cancel",
    "/accounts/:id/sync-active", "/accounts/discover", "/accounts/gmail", "/invitations",
    "/invitations/:id", "/local-access", "/logout", "/push/public-key", "/push/subscriptions", "/releases", "/rules", "/rules/:id",
    "/search", "/session", "dynamic:path"
  ].sort());
});
