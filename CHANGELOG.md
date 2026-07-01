# Changelog

[Semantic Versioning](https://semver.org). 0.x = 실험적(인터페이스 변경 가능).

## [Unreleased]
### Added
- **Events 뷰(8)**: k8s + llm-d 이벤트 통합(Warning/Normal, reason/object/message/count, 최신순). MVP1.
- (다음) PD 뷰, EPP decision score table, cache 뷰, ModelService 단위 — [ROADMAP.md](./ROADMAP.md)

## [0.2.0] — 2026-06-30
llm-d serving 운영 콘솔로 확장. 8개 뷰 + 시각화/상호작용/런처.

### Added
- 뷰 확장: **Topology**(Gateway→HTTPRoute→backend + InferencePool/EPP/SLO + KEDA autoscale + EPP-bypass 진단),
  **Perf**(구간별 지연 p50/p95/p99 · 토큰 분포 · per-model 테이블 · timeline 라인 차트), **Launch**(카탈로그 × 라이브 재고, read-only)
- **EPP Inspector**: 선택 가능 scorer + 설명 패널 + 요청 분배(scheduler_attempts) + prefix indexer
- NPU health/throttling, 오토스케일(KEDA), 절대 메모리(GB)
- 상호작용: `/` 필터 · `?` 도움말(색 범례) · `o` 정렬 · `t` 테마(default/high-contrast/colorblind) · `g` Grafana · 마우스 스크롤 · 컬럼 커스터마이징(`~/.config/lmd-top/lmd-top.yaml`)
- 디자인: 라인 차트 · 둥근 테두리 · 라이브 스피너 · 마퀴 가로스크롤 · semantic colors
- `ROADMAP.md`(LLM serving 운영 콘솔 비전), `CHANGELOG.md`

### Fixed
- 메모리 단위(RBLN/Furiosa/node DRAM 은 bytes → 항상 /1e9)
- RBLN health 반전(HEALTH=0 이 정상)
- 하이라이트 시 CJK 폭 오계산으로 글자 깨짐 → unicode-width 기반 절단 + TableState 배경 하이라이트
- Grafana(`g`)의 xdg-open 이 터미널 화면 깨뜨리던 문제 → stdio null

### Changed
- UI 전체 **영어화**

## [0.1.0] — 2026-06-30
Phase 1: 모니터링 TUI (Rust/ratatui).

### Added
- 6개 뷰: Overview/Accel/Models/EPP/Routing/Pods, 2초 자동 갱신
- 가속기(NVIDIA/RBLN/Furiosa) util/mem/temp/power, EPP scorer(ConfigMap), 모델·라우팅·파드
- 순수 Rust: Prometheus(tokio TCP HTTP/1.0) + kubectl 셸링, scale 액션
- `--snapshot` / `--render` 검증 모드

[Unreleased]: https://example/compare/v0.2.0...HEAD
[0.2.0]: https://example/compare/v0.1.0...v0.2.0
[0.1.0]: https://example/releases/tag/v0.1.0
