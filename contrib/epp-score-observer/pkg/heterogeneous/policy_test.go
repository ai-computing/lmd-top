package heterogeneous

import (
	"context"
	"encoding/json"
	fwkdl "github.com/llm-d/llm-d-router/pkg/epp/framework/interface/datalayer"
	fwksched "github.com/llm-d/llm-d-router/pkg/epp/framework/interface/scheduling"
	source "github.com/llm-d/llm-d-router/pkg/epp/framework/plugins/datalayer/source/metrics"
	dto "github.com/prometheus/client_model/go"
	"math"
	"strings"
	"testing"
	"time"
)

func ptr[T any](v T) *T { return &v }
func cfg() Config {
	return Config{MaxMetricAgeSeconds: 15, Classes: []Class{
		{Name: "gpu", Selector: map[string]string{"class": "gpu"}, Models: []string{"m"}, Capacity: 8, WaitingMetric: "wait", RunningMetric: "run"},
		{Name: "npu", Selector: map[string]string{"class": "npu"}, Models: []string{"m"}, Capacity: 4, WaitingMetric: "wait", RunningMetric: "run"},
	}}
}
func candidate(class string, w, r int) Candidate {
	return Candidate{Name: class, Labels: map[string]string{"class": class}, Waiting: ptr(w), Running: ptr(r), MetricAgeSeconds: ptr(1.0)}
}
func TestCapacityAndLoad(t *testing.T) {
	c := cfg()
	r := Request{Model: "m"}
	a := c.Evaluate(r, candidate("gpu", 8, 8))
	b := c.Evaluate(r, candidate("npu", 0, 1))
	if !a.Eligible || !b.Eligible || b.Score <= a.Score {
		t.Fatalf("busy GPU must lose to idle NPU: %+v %+v", a, b)
	}
	if c.Evaluate(r, candidate("gpu", 4, 0)).Score <= c.Evaluate(r, candidate("npu", 4, 0)).Score {
		t.Fatal("same queue should favor measured capacity")
	}
}
func TestRejections(t *testing.T) {
	c := cfg()
	r := Request{Model: "m"}
	base := candidate("npu", 0, 0)
	tests := []struct {
		name   string
		mutate func(*Candidate)
		reason string
	}{
		{"missing", func(e *Candidate) { e.Waiting = nil }, "missing-metrics"},
		{"stale", func(e *Candidate) { e.MetricAgeSeconds = ptr(16.0) }, "stale-metrics"},
		{"future", func(e *Candidate) { e.MetricAgeSeconds = ptr(-1.0) }, "invalid-metrics"},
		{"nan", func(e *Candidate) { e.MetricAgeSeconds = ptr(math.NaN()) }, "invalid-metrics"},
		{"unknown", func(e *Candidate) { e.Labels = nil }, "unknown-class"},
		{"negative", func(e *Candidate) { e.Running = ptr(-1) }, "invalid-metrics"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			e := base
			tt.mutate(&e)
			d := c.Evaluate(r, e)
			if d.Eligible || d.Reason != tt.reason {
				t.Fatalf("%+v", d)
			}
		})
	}
	if c.Evaluate(Request{Model: "other"}, base).Eligible {
		t.Fatal("model mismatch accepted")
	}
	c.Classes[1].MaxInputTokens = 100
	if c.Evaluate(r, base).Reason != "unknown-input-length" {
		t.Fatal("missing tokenization accepted")
	}
	if c.Evaluate(Request{Model: "m", InputTokens: ptr(101)}, base).Eligible {
		t.Fatal("over limit")
	}
	c.Classes[0].Selector = c.Classes[1].Selector
	if c.Evaluate(r, base).Reason != "ambiguous-class" {
		t.Fatal("overlapping class accepted")
	}
}
func TestFactoryRejectsInvalidConfig(t *testing.T) {
	for _, s := range []string{`{}`, `{"maxMetricAgeSeconds":15,"clasess":[]}`, `{"maxMetricAgeSeconds":-1,"classes":[]}`} {
		if _, err := Factory("x", json.NewDecoder(strings.NewReader(s)), nil); err == nil {
			t.Fatal(s)
		}
	}
	c := cfg()
	c.Classes[0].Capacity = math.Inf(1)
	if c.Validate() == nil {
		t.Fatal("inf")
	}
}
func gauge(v float64) *dto.MetricFamily {
	return &dto.MetricFamily{Metric: []*dto.Metric{{Gauge: &dto.Gauge{Value: &v}}}}
}
func TestExtractorAndAdapterShareDecision(t *testing.T) {
	c := cfg()
	ext := Extractor{config: c}
	plugin := Plugin{config: c}
	server := fwkdl.NewEndpoint(&fwkdl.EndpointMetadata{PodName: "npu", Labels: map[string]string{"class": "npu"}}, fwkdl.NewMetrics())
	input := fwkdl.PollInput[source.PrometheusMetricMap]{Endpoint: server, Payload: source.PrometheusMetricMap{"wait": gauge(0), "run": gauge(1)}}
	if err := ext.Extract(context.Background(), input); err != nil {
		t.Fatal(err)
	}
	ep := fwksched.NewEndpoint(server.GetMetadata(), server.GetMetrics(), server.GetAttributes())
	req := &fwksched.InferenceRequest{TargetModel: "m"}
	if len(plugin.Filter(context.Background(), req, []fwksched.Endpoint{ep})) != 1 {
		t.Fatal("fresh complete sample rejected")
	}
	if got := plugin.Score(context.Background(), req, []fwksched.Endpoint{ep})[ep]; math.Abs(got-0.8) > 1e-9 {
		t.Fatal(got)
	}
	delete(input.Payload, "wait")
	_ = ext.Extract(context.Background(), input)
	ep = fwksched.NewEndpoint(server.GetMetadata(), server.GetMetrics(), server.GetAttributes())
	if len(plugin.Filter(context.Background(), req, []fwksched.Endpoint{ep})) != 0 {
		t.Fatal("partial scrape falsely treated as zero")
	}
	ep.Put(loadKey.String(), Load{Valid: true, Updated: time.Now().Add(-time.Minute)})
	if len(plugin.Filter(context.Background(), req, []fwksched.Endpoint{ep})) != 0 {
		t.Fatal("stale sample")
	}
}
