import http from 'k6/http';
import { check, sleep } from 'k6';
import { Trend } from 'k6/metrics';

const elapsedTrend = new Trend('app_elapsed_ms');

const targetUrl = __ENV.TARGET_URL || 'http://node-api:3001';
const isWarmup = __ENV.K6_WARMUP === '1';

const payloads = [
  { userId: 21, tagText: 'class study', role: 'teacher', language: 'ko', perCategoryLimit: 20 },
  { userId: 31, tagText: 'poster design', role: 'student', language: 'ko', perCategoryLimit: 20 },
  { userId: 22, tagText: 'clean simple', role: 'all', language: 'en', perCategoryLimit: 20 },
  { userId: 41, tagText: 'chat bubble', role: 'teacher', language: 'ko', perCategoryLimit: 30 },
  { userId: 33, tagText: 'space hero', role: 'all', language: 'en', perCategoryLimit: 30 }
];

export const options = isWarmup
  ? {
      summaryTrendStats: ['avg', 'min', 'med', 'max', 'p(90)', 'p(95)', 'p(99)'],
      scenarios: {
        warmup: {
          executor: 'constant-vus',
          vus: 20,
          duration: '30s'
        }
      },
      thresholds: {
        http_req_failed: ['rate<0.01']
      }
    }
  : {
      summaryTrendStats: ['avg', 'min', 'med', 'max', 'p(90)', 'p(95)', 'p(99)'],
      scenarios: {
        node_rust_equivalent: {
          executor: 'ramping-arrival-rate',
          timeUnit: '1s',
          preAllocatedVUs: 200,
          maxVUs: 400,
          stages: [
            { duration: '1m', target: 60 },
            { duration: '3m', target: 120 },
            { duration: '3m', target: 180 },
            { duration: '1m', target: 0 }
          ]
        }
      },
      thresholds: {
        http_req_failed: ['rate<0.01'],
        http_req_duration: ['p(95)<1200', 'p(99)<2000']
      }
    };

export default function () {
  const payload = payloads[__ITER % payloads.length];
  const res = http.post(`${targetUrl}/integrated-search-like`, JSON.stringify(payload), {
    headers: { 'Content-Type': 'application/json' },
    timeout: '10s'
  });

  const ok = check(res, {
    'status is 200': (r) => r.status === 200,
    'merged exists': (r) => {
      try {
        const body = JSON.parse(r.body);
        return Array.isArray(body.merged);
      } catch {
        return false;
      }
    }
  });

  if (ok) {
    const body = JSON.parse(res.body);
    if (body?.meta?.elapsedMs) {
      elapsedTrend.add(body.meta.elapsedMs);
    }
  }

  sleep(0.2);
}
