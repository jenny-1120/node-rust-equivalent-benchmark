use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use once_cell::sync::Lazy;
use prometheus::{
    register_histogram_vec, register_int_counter_vec, Encoder, HistogramVec, IntCounterVec,
    TextEncoder,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

static REQUEST_COUNTER: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!("node_rust_equivalent_requests_total", "Total benchmark requests", &["service"])
        .expect("register counter")
});

static STAGE_DURATION_MS: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "node_rust_equivalent_stage_duration_ms",
        "Stage duration in milliseconds",
        &["service", "stage"],
        vec![1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 400.0, 800.0, 1600.0]
    )
    .expect("register stage histogram")
});

static REQUEST_DURATION_MS: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "node_rust_equivalent_request_duration_ms",
        "End-to-end request duration in milliseconds",
        &["service"],
        vec![5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 400.0, 800.0, 1600.0, 3200.0]
    )
    .expect("register request histogram")
});

const CATEGORIES: [&str; 7] = [
    "template",
    "character",
    "background",
    "effect",
    "prop",
    "speechBubble",
    "textTemplate",
];

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SeedItem {
    id: u64,
    category: String,
    role: String,
    language: String,
    title: String,
    tags: Vec<String>,
    popularity: f64,
    updated_at: String,
    owner_user_id: u64,
    purchased_by: Vec<u64>,
    #[serde(skip)]
    title_lower: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchRequest {
    user_id: u64,
    tag_text: String,
    role: String,
    language: String,
    per_category_limit: Option<usize>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchResponseItem {
    id: u64,
    category: &'static str,
    title: String,
    score: f64,
    popularity: f64,
    is_purchased: bool,
    image_url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchResponseMeta {
    service: String,
    elapsed_ms: f64,
    total_candidates: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchResponse {
    meta: SearchResponseMeta,
    merged: Vec<SearchResponseItem>,
    by_category: HashMap<&'static str, Vec<SearchResponseItem>>,
}

#[derive(Clone)]
struct AppState {
    service_name: String,
    seed: Arc<Vec<SeedItem>>,
    parallel_workers: usize,
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|x| x.parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|x| x.parse::<usize>().ok())
        .unwrap_or(default)
}

fn main() {
    let parallel_workers = env_usize("PARALLEL_WORKERS", 4).max(1);
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(parallel_workers)
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(async_main(parallel_workers));
}

async fn async_main(parallel_workers: usize) {
    let port = std::env::var("PORT")
        .ok()
        .and_then(|x| x.parse::<u16>().ok())
        .unwrap_or(3002);
    let service_name = std::env::var("SERVICE_NAME").unwrap_or_else(|_| "rust-api".to_string());
    let dataset_path = std::env::var("DATASET_PATH")
        .unwrap_or_else(|_| "/app/data/seed/integrated-search-like.json".to_string());
    let dataset_multiplier = env_u64("DATASET_MULTIPLIER", 2000).max(1);

    let raw = fs::read_to_string(dataset_path).expect("seed read failed");
    let base_seed = serde_json::from_str::<Vec<SeedItem>>(&raw).expect("seed parse failed");
    let mut seed = Vec::new();
    for idx in 0..dataset_multiplier {
        for item in &base_seed {
            let mut cloned = item.clone();
            cloned.id = item.id + idx * 100000;
            cloned.popularity = item.popularity + (idx % 10) as f64;
            cloned.title = format!("{} #{}", item.title, idx);
            cloned.title_lower = cloned.title.to_lowercase();
            seed.push(cloned);
        }
    }
    let state = AppState {
        service_name: service_name.clone(),
        seed: Arc::new(seed),
        parallel_workers,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/integrated-search-like", post(integrated_search_like))
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .expect("bind failed");
    println!(
        "{} listening on {} with {} rows and {} workers",
        service_name,
        port,
        state.seed.len(),
        parallel_workers
    );
    axum::serve(listener, app).await.expect("server failed");
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    Json(serde_json::json!({
        "ok": true,
        "service": state.service_name,
        "items": state.seed.len(),
        "parallelWorkers": state.parallel_workers
    }))
}

async fn metrics() -> impl IntoResponse {
    let mut buffer = vec![];
    let encoder = TextEncoder::new();
    let metrics = prometheus::gather();
    encoder.encode(&metrics, &mut buffer).expect("encode metrics");
    (StatusCode::OK, String::from_utf8(buffer).expect("utf8 metrics"))
}

async fn integrated_search_like(
    State(state): State<AppState>,
    Json(payload): Json<SearchRequest>,
) -> impl IntoResponse {
    if payload.user_id == 0 || payload.tag_text.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"message": "invalid payload"})),
        )
            .into_response();
    }

    let started = Instant::now();
    REQUEST_COUNTER
        .with_label_values(&[&state.service_name])
        .inc();

    match tokio::task::spawn_blocking(move || run_search(state, payload)).await {
        Ok(mut response) => {
            let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
            REQUEST_DURATION_MS
                .with_label_values(&[&response.meta.service])
                .observe(elapsed_ms);
            response.meta.elapsed_ms = elapsed_ms;
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"message": "search worker join failed"})),
        )
            .into_response(),
    }
}

fn run_search(state: AppState, payload: SearchRequest) -> SearchResponse {
    let tokens = tokenize_tag_text(&payload.tag_text);
    let per_category_limit = payload.per_category_limit.unwrap_or(30);

    let fan_out_start = Instant::now();
    let mut fan_out_result: Vec<(&'static str, Vec<(usize, f64)>)> = Vec::with_capacity(CATEGORIES.len());
    for category in CATEGORIES {
        fan_out_result.push((
            category,
            fan_out_category(
                state.seed.as_ref(),
                category,
                &payload.role,
                &payload.language,
                &tokens,
                per_category_limit,
            ),
        ));
    }
    STAGE_DURATION_MS
        .with_label_values(&[&state.service_name, "fanOutFilter"])
        .observe(fan_out_start.elapsed().as_secs_f64() * 1000.0);

    let post_process_start = Instant::now();
    let mut purchase_adjusted: Vec<(&'static str, Vec<(usize, f64, bool)>)> = Vec::new();
    purchase_adjusted.reserve(CATEGORIES.len());
    for (category, scored) in fan_out_result {
        let adjusted = scored
            .into_iter()
            .map(|(idx, score)| {
                let item = &state.seed[idx];
                let purchased = item.owner_user_id == payload.user_id
                    || item.purchased_by.contains(&payload.user_id);
                let adjusted_score = if purchased { score * 1.1 } else { score };
                (idx, adjusted_score, purchased)
            })
            .collect::<Vec<_>>();
        purchase_adjusted.push((category, adjusted));
    }
    STAGE_DURATION_MS
        .with_label_values(&[&state.service_name, "postProcess"])
        .observe(post_process_start.elapsed().as_secs_f64() * 1000.0);

    let image_url_start = Instant::now();
    let mut categorized_items: Vec<(&'static str, Vec<SearchResponseItem>)> = Vec::new();
    categorized_items.reserve(CATEGORIES.len());
    let mut merged_candidates: Vec<(usize, usize, f64)> = Vec::new();
    merged_candidates.reserve(CATEGORIES.len() * per_category_limit * 2);
    for (category, items) in purchase_adjusted {
        let mapped = items
            .into_iter()
            .map(|(idx, score, is_purchased)| {
                let item = &state.seed[idx];
                SearchResponseItem {
                    id: item.id,
                    category,
                    title: item.title.clone(),
                    score,
                    popularity: item.popularity,
                    is_purchased,
                    image_url: build_image_url(item, payload.user_id),
                }
            })
            .collect::<Vec<_>>();
        let category_idx = categorized_items.len();
        merged_candidates.extend(
            mapped
                .iter()
                .enumerate()
                .map(|(item_idx, item)| (category_idx, item_idx, item.score)),
        );
        categorized_items.push((category, mapped));
    }
    STAGE_DURATION_MS
        .with_label_values(&[&state.service_name, "imageUrlBuild"])
        .observe(image_url_start.elapsed().as_secs_f64() * 1000.0);

    let merge_start = Instant::now();
    merged_candidates.sort_by(|a, b| b.2.total_cmp(&a.2));
    merged_candidates.truncate(per_category_limit * CATEGORIES.len());

    let mut merged = Vec::with_capacity(merged_candidates.len());
    for (category_idx, item_idx, _) in merged_candidates {
        merged.push(categorized_items[category_idx].1[item_idx].clone());
    }

    let mut by_category: HashMap<&'static str, Vec<SearchResponseItem>> = HashMap::new();
    by_category.reserve(CATEGORIES.len());
    for (category, mapped) in categorized_items {
        let mut sorted = mapped;
        sorted.sort_by(|a, b| b.score.total_cmp(&a.score));
        sorted.truncate(per_category_limit);
        by_category.insert(category, sorted);
    }
    STAGE_DURATION_MS
        .with_label_values(&[&state.service_name, "mergeSort"])
        .observe(merge_start.elapsed().as_secs_f64() * 1000.0);

    SearchResponse {
        meta: SearchResponseMeta {
            service: state.service_name,
            elapsed_ms: 0.0,
            total_candidates: state.seed.len(),
        },
        merged,
        by_category,
    }
}

fn fan_out_category(
    seed: &[SeedItem],
    category: &str,
    role: &str,
    language: &str,
    tokens: &[String],
    per_category_limit: usize,
) -> Vec<(usize, f64)> {
    let mut scored: Vec<(usize, f64)> = seed
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            if item.category != category {
                return false;
            }
            if role != "all" && item.role != role {
                return false;
            }
            if item.language != language {
                return false;
            }
            tokens.iter().all(|token| {
                item.tags.iter().any(|tag| tag.contains(token))
                    || item.title_lower.contains(token)
            })
        })
        .map(|(idx, item)| (idx, score_item(item, tokens)))
        .collect();

    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.truncate(per_category_limit * 2);
    scored
}

fn tokenize_tag_text(tag_text: &str) -> Vec<String> {
    tag_text
        .to_lowercase()
        .split_whitespace()
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

fn score_item(item: &SeedItem, tokens: &[String]) -> f64 {
    let mut tag_match_count = 0.0;
    for token in tokens {
        if item.tags.iter().any(|tag| tag.contains(token)) {
            tag_match_count += 1.0;
        }
    }

    let title_bonus = if tokens
        .iter()
        .any(|token| item.title_lower.contains(token))
    {
        5.0
    } else {
        0.0
    };

    // DB 없는 벤치마크에서 날짜 파싱 비용 변수를 줄이기 위해
    // 고정 데이터의 id 기반으로 freshness 유사 점수를 만든다.
    let freshness_score = 20.0 / (1.0 + (item.id % 30) as f64);

    tag_match_count * 10.0 + item.popularity * 0.1 + title_bonus + freshness_score
}

fn build_image_url(item: &SeedItem, user_id: u64) -> String {
    let payload = format!(
        "{}:{}:{}:{}:{}",
        item.id, item.category, item.title, user_id, item.updated_at
    );
    let mut hasher = Sha256::new();
    hasher.update(payload.as_bytes());
    let digest = hasher.finalize();
    let encoded = hex::encode(digest);
    let shard = &encoded[0..2];
    format!(
        "https://cdn.local/{}/{}/{}?sig={}",
        item.category,
        shard,
        item.id,
        &encoded[0..20]
    )
}
