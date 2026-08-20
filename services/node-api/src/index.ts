import express from 'express';
import fs from 'node:fs';
import crypto from 'node:crypto';
import { Counter, Histogram, Registry, collectDefaultMetrics } from 'prom-client';

type Category =
  | 'template'
  | 'character'
  | 'background'
  | 'effect'
  | 'prop'
  | 'speechBubble'
  | 'textTemplate';

interface SeedItem {
  id: number;
  category: Category;
  role: string;
  language: string;
  title: string;
  tags: string[];
  popularity: number;
  updatedAt: string;
  ownerUserId: number;
  purchasedBy: number[];
}

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

const categories: Category[] = [
  'template',
  'character',
  'background',
  'effect',
  'prop',
  'speechBubble',
  'textTemplate'
];

const port = Number(process.env.PORT ?? 3001);
const serviceName = process.env.SERVICE_NAME ?? 'node-api';
const metricPrefix = serviceName.replace(/[^a-zA-Z0-9_]/g, '_');
const datasetPath = process.env.DATASET_PATH ?? '/app/data/seed/integrated-search-like.json';
const datasetMultiplier = Math.max(1, Number(process.env.DATASET_MULTIPLIER ?? 50));

const baseSeedData = JSON.parse(fs.readFileSync(datasetPath, 'utf-8')) as SeedItem[];
const seedData: SeedItem[] = Array.from({ length: datasetMultiplier }).flatMap((_, idx) =>
  baseSeedData.map((item) => ({
    ...item,
    id: item.id + idx * 100000,
    popularity: item.popularity + (idx % 10),
    title: `${item.title} #${idx}`
  }))
);

const registry = new Registry();
collectDefaultMetrics({ register: registry, prefix: `${metricPrefix}_` });

const requestCounter = new Counter({
  name: 'phasea_requests_total',
  help: 'Total benchmark requests',
  labelNames: ['service'],
  registers: [registry]
});

const stageDurationMs = new Histogram({
  name: 'phasea_stage_duration_ms',
  help: 'Stage duration in milliseconds',
  labelNames: ['service', 'stage'],
  buckets: [1, 2, 5, 10, 20, 50, 100, 200, 400, 800, 1600],
  registers: [registry]
});

const requestDurationMs = new Histogram({
  name: 'phasea_request_duration_ms',
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

function tokenizeTagText(tagText: string): string[] {
  return tagText
    .toLowerCase()
    .split(/\s+/)
    .map((x) => x.trim())
    .filter(Boolean);
}

function scoreItem(item: SeedItem, tokens: string[]): number {
  let tagMatchCount = 0;
  for (const token of tokens) {
    if (item.tags.some((tag) => tag.includes(token))) {
      tagMatchCount += 1;
    }
  }

  const titleBonus = tokens.some((token) => item.title.toLowerCase().includes(token)) ? 5 : 0;
  const freshnessScore = 20 / (1 + (item.id % 30));
  return tagMatchCount * 10 + item.popularity * 0.1 + titleBonus + freshnessScore;
}

function buildImageUrl(item: SeedItem, userId: number): string {
  const payload = `${item.id}:${item.category}:${item.title}:${userId}:${item.updatedAt}`;
  const digest = crypto.createHash('sha256').update(payload).digest('hex');
  const shard = digest.slice(0, 2);
  return `https://cdn.local/${item.category}/${shard}/${item.id}?sig=${digest.slice(0, 20)}`;
}

app.get('/health', (_req, res) => {
  res.json({ ok: true, service: serviceName, items: seedData.length });
});

app.get('/metrics', async (_req, res) => {
  res.set('Content-Type', registry.contentType);
  res.end(await registry.metrics());
});

app.post('/integrated-search-like', async (req, res) => {
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
    categories.map(async (category) => {
      const filtered = seedData.filter((item) => {
        if (item.category !== category) return false;
        if (role !== 'all' && item.role !== role) return false;
        if (item.language !== language) return false;
        return tokens.every(
          (token) =>
            item.tags.some((tag) => tag.includes(token)) ||
            item.title.toLowerCase().includes(token)
        );
      });

      const scored = filtered
        .map((item) => ({ item, score: scoreItem(item, tokens) }))
        .sort((a, b) => b.score - a.score)
        .slice(0, perCategoryLimit * 2);
      return { category, scored };
    })
  );
  stageDurationMs.observe({ service: serviceName, stage: 'fanOutFilter' }, nowMs() - tFanOutStart);

  const tPurchaseStart = nowMs();
  const purchaseAdjusted = fanOutResult.map(({ category, scored }) => {
    const adjusted = scored.map(({ item, score }) => {
      const isPurchased = item.purchasedBy.includes(userId) || item.ownerUserId === userId;
      return { category, item, score: isPurchased ? score * 1.1 : score, isPurchased };
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
    meta: { service: serviceName, elapsedMs: elapsed, totalCandidates: seedData.length },
    merged,
    byCategory
  });
});

app.listen(port, () => {
  // eslint-disable-next-line no-console
  console.log(`${serviceName} listening on ${port} with ${seedData.length} rows`);
});
