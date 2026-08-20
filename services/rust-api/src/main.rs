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
    category: String,
    title: String,
    score: f64,
    popularity: f64,
    is_purchased: bool,
    image_url: String,
}

#[derive(Clone)]
struct AppState {
    service_name: String,
    seed: Arc<Vec<SeedItem>>,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let port = std::env::var("PORT")
        .ok()
        .and_then(|x| x.parse::<u16>().ok())
        .unwrap_or(3002);
    let service_name = std::env::var("SERVICE_NAME").unwrap_or_else(|_| "rust-api".to_string());
    let dataset_path = std::env::var("DATASET_PATH")
        .unwrap_or_else(|_| "/app/data/seed/integrated-search-like.json".to_string());
    let dataset_multiplier = std::env::var("DATASET_MULTIPLIER")
        .ok()
        .and_then(|x| x.parse::<u64>().ok())
        .unwrap_or(50)
        .max(1);

    let raw = fs::read_to_string(dataset_path).expect("seed read failed");
    let base_seed = serde_json::from_str::<Vec<SeedItem>>(&raw).expect("seed parse failed");
    let mut seed = Vec::new();
    for idx in 0..dataset_multiplier {
        for item in &base_seed {
            let mut cloned = item.clone();
            cloned.id = item.id + idx * 100000;
            cloned.popularity = item.popularity + (idx % 10) as f64;
            cloned.title = format!("{} #{}", item.title, idx);
            seed.push(cloned);
        }
    }
    let state = AppState {
        service_name: service_name.clone(),
        seed: Arc::new(seed),
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
        "{} listening on {} with {} rows",
        service_name,
        port,
        state.seed.len()
    );
    axum::serve(listener, app).await.expect("server failed");
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    Json(serde_json::json!({
        "ok": true,
        "service": state.service_name,
        "items": state.seed.len()
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

    let tokens = tokenize_tag_text(&payload.tag_text);
    let per_category_limit = payload.per_category_limit.unwrap_or(30);

    let fan_out_start = Instant::now();
    let mut handles = Vec::new();
    for category in CATEGORIES {
        let seed = Arc::clone(&state.seed);
        let role = payload.role.clone();
        let language = payload.language.clone();
        let tokens_copy = tokens.clone();
        let category_name = category.to_string();

        let handle = tokio::spawn(async move {
            let mut scored: Vec<(SeedItem, f64)> = seed
                .iter()
                .filter(|item| {
                    if item.category != category_name {
                        return false;
                    }
                    if role != "all" && item.role != role {
                        return false;
                    }
                    if item.language != language {
                        return false;
                    }
                    tokens_copy.iter().all(|token| {
                        item.tags.iter().any(|tag| tag.contains(token))
                            || item.title.to_lowercase().contains(token)
                    })
                })
                .map(|item| (item.clone(), score_item(item, &tokens_copy)))
                .collect();

            scored.sort_by(|a, b| b.1.total_cmp(&a.1));
            scored.truncate(per_category_limit * 2);
            (category_name, scored)
        });

        handles.push(handle);
    }

    let mut fan_out_result: Vec<(String, Vec<(SeedItem, f64)>)> = Vec::new();
    for handle in handles {
        let result = handle.await.expect("join failed");
        fan_out_result.push(result);
    }
    STAGE_DURATION_MS
        .with_label_values(&[&state.service_name, "fanOutFilter"])
        .observe(fan_out_start.elapsed().as_secs_f64() * 1000.0);

    let post_process_start = Instant::now();
    let mut purchase_adjusted: Vec<(String, Vec<(SeedItem, f64, bool)>)> = Vec::new();
    for (category, scored) in fan_out_result {
        let adjusted = scored
            .into_iter()
            .map(|(item, score)| {
                let purchased =
                    item.owner_user_id == payload.user_id || item.purchased_by.contains(&payload.user_id);
                let adjusted_score = if purchased { score * 1.1 } else { score };
                (item, adjusted_score, purchased)
            })
            .collect::<Vec<_>>();
        purchase_adjusted.push((category, adjusted));
    }
    STAGE_DURATION_MS
        .with_label_values(&[&state.service_name, "postProcess"])
        .observe(post_process_start.elapsed().as_secs_f64() * 1000.0);

    let image_url_start = Instant::now();
    let mut with_url: Vec<(String, Vec<SearchResponseItem>)> = Vec::new();
    for (category, items) in purchase_adjusted {
        let mapped = items
            .into_iter()
            .map(|(item, score, is_purchased)| SearchResponseItem {
                id: item.id,
                category: category.clone(),
                title: item.title.clone(),
                score,
                popularity: item.popularity,
                is_purchased,
                image_url: build_image_url(&item, payload.user_id),
            })
            .collect::<Vec<_>>();
        with_url.push((category, mapped));
    }
    STAGE_DURATION_MS
        .with_label_values(&[&state.service_name, "imageUrlBuild"])
        .observe(image_url_start.elapsed().as_secs_f64() * 1000.0);

    let merge_start = Instant::now();
    let mut by_category: HashMap<String, Vec<SearchResponseItem>> = HashMap::new();
    let mut merged = with_url
        .iter()
        .flat_map(|(_, v)| v.clone())
        .collect::<Vec<SearchResponseItem>>();
    merged.sort_by(|a, b| b.score.total_cmp(&a.score));
    merged.truncate(per_category_limit * CATEGORIES.len());

    for (category, mapped) in with_url {
        let mut sorted = mapped;
        sorted.sort_by(|a, b| b.score.total_cmp(&a.score));
        sorted.truncate(per_category_limit);
        by_category.insert(category, sorted);
    }
    STAGE_DURATION_MS
        .with_label_values(&[&state.service_name, "mergeSort"])
        .observe(merge_start.elapsed().as_secs_f64() * 1000.0);

    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    REQUEST_DURATION_MS
        .with_label_values(&[&state.service_name])
        .observe(elapsed_ms);

    let response = serde_json::json!({
        "meta": {
            "service": state.service_name,
            "elapsedMs": elapsed_ms,
            "totalCandidates": state.seed.len()
        },
        "merged": merged,
        "byCategory": by_category
    });

    (StatusCode::OK, Json(response)).into_response()
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
        .any(|token| item.title.to_lowercase().contains(token))
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

