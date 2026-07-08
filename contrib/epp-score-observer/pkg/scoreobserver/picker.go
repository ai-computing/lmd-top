// Package scoreobserver is an EPP (llm-d endpoint-picker) Picker plugin that
// exports the per-endpoint composite scheduler score as a Prometheus metric,
// then delegates the actual selection to the standard max-score-picker.
//
// Why a Picker: the framework runs all Scorers, sums their weighted outputs
// into ScoredEndpoint.Score, and hands the full candidate list to the Picker
// (see pkg/epp/framework/interface/scheduling: Picker.Pick([]*ScoredEndpoint)).
// So the Picker is the one place that sees the real, final per-endpoint score
// the router used — exactly the "why this pod?" signal lmd-top wants.
//
// Metrics land on handle.Metrics() = the controller-runtime registry that the
// EPP already serves on :9090 (runner wires WithMetricsRecorder(ctrlmetrics.Registry)),
// so no extra endpoint/scrape config is needed beyond the existing EPP ServiceMonitor.
package scoreobserver

import (
	"context"
	"encoding/json"
	"sync"

	"github.com/prometheus/client_golang/prometheus"

	fwkplugin "github.com/llm-d/llm-d-router/pkg/epp/framework/interface/plugin"
	fwksched "github.com/llm-d/llm-d-router/pkg/epp/framework/interface/scheduling"
	"github.com/llm-d/llm-d-router/pkg/epp/framework/plugins/scheduling/picker/maxscore"
)

// PluginType is the value referenced from the EPP config file's schedulingProfiles.
const PluginType = "endpoint-score-observer"

var (
	// epp_endpoint_score: last composite score each candidate endpoint received.
	// Higher = more preferred. Labeled by pod (and namespace/pool) so lmd-top can
	// join it to its endpoint table. Gauge = "current view of the last cycle".
	scoreGauge = prometheus.NewGaugeVec(prometheus.GaugeOpts{
		Name: "epp_endpoint_score",
		Help: "Composite scheduler score per candidate endpoint from the most recent scheduling cycle (higher = preferred).",
	}, []string{"pod", "namespace", "pool"})

	// epp_endpoint_picked_total: cumulative picks per endpoint (companion to the
	// existing scheduler_attempts metric, but keyed the same way as the score).
	pickedCounter = prometheus.NewCounterVec(prometheus.CounterOpts{
		Name: "epp_endpoint_picked_total",
		Help: "Cumulative number of times each endpoint was picked by the scheduler.",
	}, []string{"pod", "namespace", "pool"})

	registerOnce sync.Once
)

var _ fwksched.Picker = (*Observer)(nil) // compile-time interface check (verified: builds against llm-d-router@main)

// Factory is registered under PluginType. It registers the metrics on the EPP's
// scraped registry (via handle.Metrics()) and returns a Picker that observes then delegates.
func Factory(name string, _ *json.Decoder, handle fwkplugin.Handle) (fwkplugin.Plugin, error) {
	registerOnce.Do(func() {
		if rec := handle.Metrics(); rec != nil {
			// Ignore AlreadyRegistered — harmless if the process re-instantiates the plugin.
			_ = rec.Register(scoreGauge)
			_ = rec.Register(pickedCounter)
		}
	})
	// Delegate real selection to the stock max-score picker (default max-num-of-endpoints).
	inner := maxscore.NewMaxScorePicker(0).WithName(name)
	return &Observer{
		typedName: fwkplugin.TypedName{Type: PluginType, Name: name},
		inner:     inner,
		pool:      handleNamespaceGuess(handle),
	}, nil
}

// Observer wraps max-score-picker and exports per-endpoint scores.
type Observer struct {
	typedName fwkplugin.TypedName
	inner     *maxscore.MaxScorePicker
	pool      string
}

// TypedName satisfies plugin.Plugin.
func (o *Observer) TypedName() fwkplugin.TypedName { return o.typedName }

// Pick exports each candidate's score, then returns the max-score selection unchanged.
func (o *Observer) Pick(ctx context.Context, scored []*fwksched.ScoredEndpoint) *fwksched.ProfileRunResult {
	for _, se := range scored {
		if se == nil {
			continue
		}
		md := se.GetMetadata()
		if md == nil || md.PodName == "" {
			continue
		}
		scoreGauge.WithLabelValues(md.PodName, md.NamespacedName.Namespace, o.pool).Set(se.Score)
	}
	res := o.inner.Pick(ctx, scored)
	if res != nil {
		for _, ep := range res.TargetEndpoints {
			if md := ep.GetMetadata(); md != nil && md.PodName != "" {
				pickedCounter.WithLabelValues(md.PodName, md.NamespacedName.Namespace, o.pool).Inc()
			}
		}
	}
	return res
}

// handleNamespaceGuess derives a coarse "pool" label from the handle's pod list namespace.
// Best-effort only (empty is fine); the pod label is the join key lmd-top actually uses.
func handleNamespaceGuess(handle fwkplugin.Handle) string {
	for _, nn := range handle.PodList() {
		if nn.Namespace != "" {
			return nn.Namespace
		}
	}
	return ""
}
