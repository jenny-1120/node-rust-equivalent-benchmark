import { parentPort, workerData } from 'node:worker_threads';
import { fanOutCategory, loadSeed, type CategoryJob } from './search-core.js';

if (!parentPort) {
  throw new Error('search-worker must run as a worker thread');
}

const port = parentPort;
const { datasetPath, datasetMultiplier } = workerData as {
  datasetPath: string;
  datasetMultiplier: number;
};

const seed = loadSeed(datasetPath, datasetMultiplier);
port.postMessage({ type: 'ready', items: seed.length });

port.on('message', (job: CategoryJob) => {
  port.postMessage({
    type: 'result',
    category: job.category,
    scored: fanOutCategory(seed, job)
  });
});
