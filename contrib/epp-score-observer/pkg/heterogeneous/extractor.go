package heterogeneous

import (
	"context"
	"encoding/json"
	"fmt"
	fwkdl "github.com/llm-d/llm-d-router/pkg/epp/framework/interface/datalayer"
	fwkplugin "github.com/llm-d/llm-d-router/pkg/epp/framework/interface/plugin"
	source "github.com/llm-d/llm-d-router/pkg/epp/framework/plugins/datalayer/source/metrics"
	"math"
	"time"
)

const ExtractorType = "heterogeneous-load-extractor"

var loadKey = fwkplugin.NewDataKey("heterogeneous-load", ExtractorType)

// Keep an independent timestamp: unrelated KV updates cannot refresh missing load.
type Load struct {
	Waiting, Running int
	Updated          time.Time
	Valid            bool
}

func (l Load) Clone() fwkdl.Cloneable { return l }

type Extractor struct {
	name   fwkplugin.TypedName
	config Config
}

func ExtractorFactory(name string, d *json.Decoder, h fwkplugin.Handle) (fwkplugin.Plugin, error) {
	if name != ExtractorType {
		return nil, fmt.Errorf("load extractor must be named %s", ExtractorType)
	}
	p, err := Factory(name, d, h)
	if err != nil {
		return nil, err
	}
	return &Extractor{name: fwkplugin.TypedName{Type: ExtractorType, Name: name}, config: p.(*Plugin).config}, nil
}
func (e *Extractor) TypedName() fwkplugin.TypedName { return e.name }
func (e *Extractor) Produces() map[fwkplugin.DataKey]any {
	return map[fwkplugin.DataKey]any{loadKey: Load{}}
}

// Exact single-series gauges only. Ambiguous multi-model/DP series need an explicit adapter.
func count(m source.PrometheusMetricMap, name string) (int, bool) {
	family := m[name]
	if family == nil || len(family.Metric) != 1 {
		return 0, false
	}
	gauge := family.Metric[0].GetGauge()
	if gauge == nil {
		return 0, false
	}
	n := gauge.GetValue()
	if !finite(n) || n < 0 || n >= float64(math.MaxInt) || math.Trunc(n) != n {
		return 0, false
	}
	return int(n), true
}
func (e *Extractor) Extract(_ context.Context, in fwkdl.PollInput[source.PrometheusMetricMap]) error {
	if in.Endpoint == nil {
		return fmt.Errorf("missing endpoint")
	}
	load := Load{}
	defer func() { in.Endpoint.GetAttributes().Put(loadKey.String(), load) }()
	md := in.Endpoint.GetMetadata()
	if md == nil {
		return nil
	}
	var chosen *Class
	for i := range e.config.Classes {
		cl := &e.config.Classes[i]
		match := true
		for k, v := range cl.Selector {
			if md.Labels[k] != v {
				match = false
				break
			}
		}
		if match {
			if chosen != nil {
				return nil
			}
			chosen = cl
		}
	}
	if chosen == nil {
		return nil
	}
	w, wok := count(in.Payload, chosen.WaitingMetric)
	r, rok := count(in.Payload, chosen.RunningMetric)
	if wok && rok {
		load = Load{Waiting: w, Running: r, Updated: time.Now(), Valid: true}
	}
	return nil
}
