package scoreobserver

import (
	"context"
	"testing"

	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/client_golang/prometheus/testutil"
	"k8s.io/apimachinery/pkg/types"

	fwkdl "github.com/llm-d/llm-d-router/pkg/epp/framework/interface/datalayer"
	fwkplugin "github.com/llm-d/llm-d-router/pkg/epp/framework/interface/plugin"
	fwksched "github.com/llm-d/llm-d-router/pkg/epp/framework/interface/scheduling"
)

func ep(pod, ns string) fwksched.Endpoint {
	meta := &fwkdl.EndpointMetadata{
		PodName:        pod,
		NamespacedName: types.NamespacedName{Namespace: ns, Name: pod},
	}
	return fwksched.NewEndpoint(meta, nil, fwkdl.NewAttributes())
}

// The picker exports each candidate's composite score as a gauge, then delegates
// to max-score (highest score wins). This is the core contract lmd-top relies on.
func TestObserverExportsScoresAndPicksMax(t *testing.T) {
	reg := prometheus.NewRegistry()
	handle := fwkplugin.NewEppHandle(context.Background(),
		func() []types.NamespacedName { return []types.NamespacedName{{Namespace: "llm-serving"}} },
		fwkplugin.WithMetricsRecorder(reg))

	p, err := Factory("obs", nil, handle)
	if err != nil {
		t.Fatalf("factory: %v", err)
	}
	obs, ok := p.(*Observer)
	if !ok {
		t.Fatalf("expected *Observer, got %T", p)
	}

	scored := []*fwksched.ScoredEndpoint{
		{Endpoint: ep("podA", "llm-serving"), Score: 0.9},
		{Endpoint: ep("podB", "llm-serving"), Score: 0.3},
	}
	res := obs.Pick(context.Background(), scored)

	// Gauge reflects the real scores per pod.
	if got := testutil.ToFloat64(scoreGauge.WithLabelValues("podA", "llm-serving", "llm-serving")); got != 0.9 {
		t.Errorf("podA score gauge = %v, want 0.9", got)
	}
	if got := testutil.ToFloat64(scoreGauge.WithLabelValues("podB", "llm-serving", "llm-serving")); got != 0.3 {
		t.Errorf("podB score gauge = %v, want 0.3", got)
	}

	// Delegated selection picks the highest-scored endpoint (podA).
	if res == nil || len(res.TargetEndpoints) == 0 {
		t.Fatalf("no endpoints picked")
	}
	if md := res.TargetEndpoints[0].GetMetadata(); md == nil || md.PodName != "podA" {
		t.Errorf("picked %v, want podA (max score)", res.TargetEndpoints[0].GetMetadata())
	}

	// The picked endpoint's counter incremented.
	if got := testutil.ToFloat64(pickedCounter.WithLabelValues("podA", "llm-serving", "llm-serving")); got != 1 {
		t.Errorf("podA picked counter = %v, want 1", got)
	}
}
