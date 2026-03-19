import { runLivePreflight } from "../driver/preflight";

const result = runLivePreflight({
	sourceStateDir: process.env.SILO_E2E_SOURCE_STATE_DIR,
});

console.log("Live e2e preflight passed");
console.log(`source state: ${result.sourceStateDir}`);
console.log(`gcloud account: ${result.activeAccount}`);
console.log(`gcloud project: ${result.activeProject}`);
