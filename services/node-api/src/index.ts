import crypto from 'node:crypto';
import { Worker } from 'node:worker_threads';
import express from 'express';
import { Counter, Histogram, Registry, collectDefaultMetrics } from 'prom-client';
import {
  categories,
  tokenizeTagText,
  type Category,
  type CategoryJob,
  type FanOutHit
} from './search-core.js';

interface SearchRequest {
  userId: number;
  tagText: string;
  role: string;
  language: string;
  perCategoryLimit?: number;
}

interface SearchResponseItem {
  id: number;
  category: Category;
  title: string;
  score: number;
  popularity: number;
  isPurchased: boolean;
  imageUrl: string;
}

interface WorkerResultMessage {
  type: 'result';
  category: Category;
  scored: FanOutHit[];
}

interface WorkerReadyMessage {
  type: 'ready';
  items: number;
}

type WorkerMessage = WorkerResultMessage | WorkerReadyMessage;

interface PendingJob {
  job: CategoryJob;
  resolve: (value: { category: Category; scored: FanOutHit[] }) => void;
  reject: (err: Error) => void;
}

class SearchWorkerPool {
  private readonly idle: Worker[] = [];
  private readonly queue: PendingJob[] = [];

  static async create(
    size: number,
    datasetPath: string,
    datasetMultiplier: number
  ): Promise<{ pool: SearchWorkerPool; items: number }> {
    const pool = new SearchWorkerPool();
    const workers = await Promise.all(
      Array.from({ length: size }, () =>
        SearchWorkerPool.spawnWorker(datasetPath, datasetMultiplier)
      )
    );
    pool.idle.push(...workers.map((entry) => entry.worker));
    return { pool, items: workers[0]?.items ?? 0 };
  }

  private static spawnWorker(
    datasetPath: string,
    datasetMultiplier: number
  ): Promise<{ worker: Worker; items: number }> {
    return new Promise((resolve, reject) => {
      const worker = new Worker(new URL('./search-worker.js', import.meta.url), {
        workerData: { datasetPath, datasetMultiplier }
      });
      const onError = (err: Error) => reject(err);
      const onExit = (code: number) => {
        if (code !== 0) reject(new Error(`search worker exited with code ${code}`));
      };
      worker.once('error', onError);
      worker.once('exit', onExit);
      worker.once('message', (msg: WorkerMessage) => {
        worker.off('error', onError);
        worker.off('exit', onExit);
        if (msg.type === 'ready') {
          resolve({ worker, items: msg.items });
          return;
        }
        reject(new Error('search worker did not send ready'));
      });
    });
  }

  run(job: CategoryJob): Promise<{ category: Category; scored: FanOutHit[] }> {
    return new Promise((resolve, reject) => {
      const pending: PendingJob = { job, resolve, reject };
      const worker = this.idle.pop();
      if (worker) this.dispatch(worker, pending);
      else this.queue.push(pending);
    });
  }

  private dispatch(worker: Worker, pending: PendingJob): void {
    const onMessage = (msg: WorkerMessage) => {
      cleanup();
      this.release(worker);
      if (msg.type === 'result') {
        pending.resolve({ category: msg.category, scored: msg.scored });
        return;
      }
      pending.reject(new Error('unexpected worker message'));
    };
    const onError = (err: Error) => {
      cleanup();
      pending.reject(err);
    };
    const cleanup = () => {
      worker.off('message', onMessage);
      worker.off('error', onError);
    };
    worker.once('message', onMessage);
    worker.once('error', onError);
    worker.postMessage(pending.job);
  }

  private release(worker: Worker): void {
    const next = this.queue.shift();
    if (next) this.dispatch(worker, next);
    else this.idle.push(worker);
  }
}

const port = Number(process.env.PORT ?? 3001);
const serviceName = process.env.SERVICE_NAME ?? 'node-api';
const metricPrefix = serviceName.replace(/[^a-zA-Z0-9_]/g, '_');
const datasetPath = process.env.DATASET_PATH ?? '/app/data/seed/integrated-search-like.json';
const datasetMultiplier = Math.max(1, Number(process.env.DATASET_MULTIPLIER ?? 2000));
const parallelWorkers = Math.max(1, Number(process.env.PARALLEL_WORKERS ?? 4));

const registry = new Registry();
collectDefaultMetrics({ register: registry, prefix: `${metricPrefix}_` });

const requestCounter = new Counter({
  name: 'node_rust_equivalent_requests_total',
  help: 'Total benchmark requests',
  labelNames: ['service'],
  registers: [registry]
});

const stageDurationMs = new Histogram({
  name: 'node_rust_equivalent_stage_duration_ms',
  help: 'Stage duration in milliseconds',
  labelNames: ['service', 'stage'],
  buckets: [1, 2, 5, 10, 20, 50, 100, 200, 400, 800, 1600],
  registers: [registry]
});

const requestDurationMs = new Histogram({
  name: 'node_rust_equivalent_request_duration_ms',
  help: 'End-to-end request duration in milliseconds',
  labelNames: ['service'],
  buckets: [5, 10, 20, 50, 100, 200, 400, 800, 1600, 3200],
  registers: [registry]
});

const app = express();
app.use(express.json({ limit: '1mb' }));

function nowMs(): number {
  return Number(process.hrtime.bigint() / 1_000_000n);
}

function buildImageUrl(
  item: Pick<FanOutHit, 'id' | 'category' | 'title' | 'updatedAt'>,
  userId: number
): string {
  const payload = `${item.id}:${item.category}:${item.title}:${userId}:${item.updatedAt}`;
  const digest = crypto.createHash('sha256').update(payload).digest('hex');
  const shard = digest.slice(0, 2);
  return `https://cdn.local/${item.category}/${shard}/${item.id}?sig=${digest.slice(0, 20)}`;
}

const workerPoolReady = SearchWorkerPool.create(parallelWorkers, datasetPath, datasetMultiplier);
let workerPool: SearchWorkerPool | undefined;
let seedItemCount = 0;

app.get('/health', (_req, res) => {
  res.json({
    ok: Boolean(workerPool),
    service: serviceName,
    items: seedItemCount,
    parallelWorkers
  });
});

app.get('/metrics', async (_req, res) => {
  res.set('Content-Type', registry.contentType);
  res.end(await registry.metrics());
});

app.post('/integrated-search-like', async (req, res) => {
  if (!workerPool) {
    return res.status(503).json({ message: 'workers not ready' });
  }

  const startedAt = nowMs();
  requestCounter.inc({ service: serviceName });

  const body = req.body as SearchRequest;
  const userId = Number(body.userId ?? 0);
  const tagText = String(body.tagText ?? '').trim();
  const role = String(body.role ?? 'all');
  const language = String(body.language ?? 'en');
  const perCategoryLimit = Number(body.perCategoryLimit ?? 30);

  if (!tagText || !Number.isFinite(userId)) {
    return res.status(400).json({ message: 'invalid payload' });
  }

  const tokens = tokenizeTagText(tagText);

  const tFanOutStart = nowMs();
  const fanOutResult = await Promise.all(
    categories.map((category) =>
      workerPool!.run({
        category,
        tokens,
        role,
        language,
        perCategoryLimit
      })
    )
  );
  stageDurationMs.observe({ service: serviceName, stage: 'fanOutFilter' }, nowMs() - tFanOutStart);

  const tPurchaseStart = nowMs();
  const purchaseAdjusted = fanOutResult.map(({ category, scored }) => {
    const adjusted = scored.map((item) => {
      const isPurchased = item.purchasedBy.includes(userId) || item.ownerUserId === userId;
      return { category, item, score: isPurchased ? item.score * 1.1 : item.score, isPurchased };
    });
    return { category, adjusted };
  });
  stageDurationMs.observe(
    { service: serviceName, stage: 'postProcess' },
    nowMs() - tPurchaseStart
  );

  const tUrlStart = nowMs();
  const withUrl = purchaseAdjusted.map(({ category, adjusted }) => {
    const mapped = adjusted.map(({ item, score, isPurchased }) => ({
      id: item.id,
      category,
      title: item.title,
      score,
      popularity: item.popularity,
      isPurchased,
      imageUrl: buildImageUrl(item, userId)
    }));
    return { category, mapped };
  });
  stageDurationMs.observe(
    { service: serviceName, stage: 'imageUrlBuild' },
    nowMs() - tUrlStart
  );

  const tMergeStart = nowMs();
  const byCategory: Record<string, SearchResponseItem[]> = {};
  const merged = withUrl
    .flatMap(({ mapped }) => mapped)
    .sort((a, b) => b.score - a.score)
    .slice(0, perCategoryLimit * categories.length);

  for (const { category, mapped } of withUrl) {
    byCategory[category] = mapped.sort((a, b) => b.score - a.score).slice(0, perCategoryLimit);
  }
  stageDurationMs.observe({ service: serviceName, stage: 'mergeSort' }, nowMs() - tMergeStart);

  const elapsed = nowMs() - startedAt;
  requestDurationMs.observe({ service: serviceName }, elapsed);

  return res.json({
    meta: { service: serviceName, elapsedMs: elapsed, totalCandidates: seedItemCount },
    merged,
    byCategory
  });
});

workerPoolReady
  .then(({ pool, items }) => {
    workerPool = pool;
    seedItemCount = items;
    app.listen(port, () => {
      // eslint-disable-next-line no-console
      console.log(
        `${serviceName} listening on ${port} with ${seedItemCount} rows and ${parallelWorkers} workers`
      );
    });
  })
  .catch((err) => {
    console.error('failed to start search workers', err);
    process.exit(1);
  });
