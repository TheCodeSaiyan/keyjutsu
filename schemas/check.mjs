// Validates the plan schemas against their fixtures. Files under
// examples/valid must pass the proposal schema and files under
// examples/invalid must fail it: a fixture that stops failing means the schema
// has been loosened, which is exactly what this check exists to catch.
//
// Files under examples/structure-invalid must *pass* the schema. They are
// wrong in ways JSON Schema cannot see (cycles, dangling references), and the
// Rust plan crate refuses them; their passing here is what shows those checks
// are needed.
import Ajv2020 from "ajv/dist/2020.js";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const dir = join(dirname(fileURLToPath(import.meta.url)), "plan", "v1");
const load = (p) => JSON.parse(readFileSync(p, "utf8"));

const ajv = new Ajv2020({
  allErrors: true,
  strict: true,
  strictRequired: false,
  allowUnionTypes: true,
});
ajv.addSchema(load(join(dir, "plan.schema.json")));
const proposal = ajv.compile(load(join(dir, "proposal.schema.json")));
const plan = ajv.getSchema("urn:keyjutsu:schema:plan:1.0");

let failures = 0;
for (const kind of ["valid", "invalid", "structure-invalid"]) {
  for (const file of readdirSync(join(dir, "examples", kind)).sort()) {
    const ok = proposal(load(join(dir, "examples", kind, file)));
    const expected = kind !== "invalid";
    const mark = ok === expected ? "ok  " : "FAIL";
    if (ok !== expected) failures++;
    console.log(
      `${mark} ${kind}/${file}${!ok && expected ? ` ${ajv.errorsText(proposal.errors)}` : ""}`,
    );
  }
}

// A stored plan may carry KeyJutsu's own section; a proposal may not.
const stored = load(join(dir, "examples", "invalid", "agent-claims-readiness.json"));
if (!plan(stored)) {
  failures++;
  console.log(`FAIL stored plan with keyjutsu section rejected: ${ajv.errorsText(plan.errors)}`);
} else {
  console.log("ok   stored plans may carry the keyjutsu section");
}

if (failures) {
  console.error(`${failures} schema check(s) failed`);
  process.exit(1);
}
