package heterogeneous

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"time"

	fwkdl "github.com/llm-d/llm-d-router/pkg/epp/framework/interface/datalayer"
	fwkplugin "github.com/llm-d/llm-d-router/pkg/epp/framework/interface/plugin"
	fwksched "github.com/llm-d/llm-d-router/pkg/epp/framework/interface/scheduling"
)

var _ fwksched.Filter = (*Plugin)(nil)
var _ fwksched.Scorer = (*Plugin)(nil)

type Plugin struct {
	name   fwkplugin.TypedName
	config Config
}

func Factory(name string, dec *json.Decoder, _ fwkplugin.Handle) (fwkplugin.Plugin, error) {
	if dec == nil {
		return nil, fmt.Errorf("heterogeneous-capacity requires parameters")
	}
	dec.DisallowUnknownFields()
	var cfg Config
	if err := dec.Decode(&cfg); err != nil {
		return nil, err
	}
	if err := dec.Decode(new(any)); err != io.EOF {
		return nil, fmt.Errorf("parameters must contain exactly one JSON object")
	}
	if err := cfg.Validate(); err != nil {
		return nil, err
	}
	return &Plugin{name: fwkplugin.TypedName{Type: PluginType, Name: name}, config: cfg}, nil
}
func (p *Plugin) TypedName() fwkplugin.TypedName    { return p.name }
func (p *Plugin) Category() fwksched.ScorerCategory { return fwksched.Distribution }
func (p *Plugin) Consumes() fwkplugin.DataDependencies {
	return fwkplugin.DataDependencies{Required: map[fwkplugin.DataKey]any{loadKey: Load{}}}
}
func decisionInput(req *fwksched.InferenceRequest, ep fwksched.Endpoint, now time.Time) (Request, Candidate) {
	r := Request{}
	e := Candidate{}
	if req != nil {
		r.Model = req.TargetModel
		if req.Body != nil && req.Body.TokenizedPrompt != nil {
			n := req.Body.TokenizedPrompt.TokenCount()
			r.InputTokens = &n
		}
	}
	if ep == nil {
		return r, e
	}
	if md := ep.GetMetadata(); md != nil {
		e.Name = md.NamespacedName.Namespace + "/" + md.PodName + ":" + md.Port
		e.Labels = md.Labels
	}
	if load, ok := fwkdl.ReadAttribute[Load](ep, loadKey.String()); ok && load.Valid {
		waiting, running := load.Waiting, load.Running
		age := now.Sub(load.Updated).Seconds()
		e.Waiting = &waiting
		e.Running = &running
		e.MetricAgeSeconds = &age
	}

	return r, e
}
func (p *Plugin) Filter(_ context.Context, req *fwksched.InferenceRequest, eps []fwksched.Endpoint) []fwksched.Endpoint {
	result := make([]fwksched.Endpoint, 0, len(eps))
	now := time.Now()
	for _, ep := range eps {
		r, e := decisionInput(req, ep, now)
		if p.config.Evaluate(r, e).Eligible {
			result = append(result, ep)
		}
	}
	return result
}
func (p *Plugin) Score(_ context.Context, req *fwksched.InferenceRequest, eps []fwksched.Endpoint) map[fwksched.Endpoint]float64 {
	result := make(map[fwksched.Endpoint]float64, len(eps))
	now := time.Now()
	for _, ep := range eps {
		r, e := decisionInput(req, ep, now)
		result[ep] = p.config.Evaluate(r, e).Score
	}
	return result
}
