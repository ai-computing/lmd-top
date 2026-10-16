// Package heterogeneous keeps the routing decision pure, independently testable,
// and shared by the real EPP adapter and the offline replay command.
package heterogeneous

import (
	"fmt"
	"math"
	"strings"
)

const PluginType = "heterogeneous-capacity"

type Class struct {
	WaitingMetric  string            `json:"waitingMetric"`
	RunningMetric  string            `json:"runningMetric"`
	Name           string            `json:"name"`
	Selector       map[string]string `json:"selector"`
	Models         []string          `json:"models"`
	Capacity       float64           `json:"capacity"`                 // measured completed requests/s for this workload cohort
	MaxInputTokens int               `json:"maxInputTokens,omitempty"` // 0 disables the optional length guard
}
type Config struct {
	MaxMetricAgeSeconds float64 `json:"maxMetricAgeSeconds"`
	Classes             []Class `json:"classes"`
}

func (c Config) Validate() error {
	if !finite(c.MaxMetricAgeSeconds) || c.MaxMetricAgeSeconds <= 0 {
		return fmt.Errorf("maxMetricAgeSeconds must be positive and finite")
	}
	if len(c.Classes) == 0 {
		return fmt.Errorf("classes must not be empty")
	}
	names := map[string]bool{}
	for _, cl := range c.Classes {
		if strings.TrimSpace(cl.Name) == "" || names[cl.Name] {
			return fmt.Errorf("class names must be nonempty and unique")
		}
		names[cl.Name] = true
		if !finite(cl.Capacity) || cl.Capacity <= 0 {
			return fmt.Errorf("%s: capacity must be positive measured requests/s", cl.Name)
		}
		if cl.WaitingMetric == "" || cl.RunningMetric == "" {
			return fmt.Errorf("%s: waitingMetric and runningMetric required", cl.Name)
		}
		if cl.MaxInputTokens < 0 || len(cl.Selector) == 0 || len(cl.Models) == 0 {
			return fmt.Errorf("%s: selector/models required; maxInputTokens must be nonnegative", cl.Name)
		}
		for k, v := range cl.Selector {
			if strings.TrimSpace(k) == "" || strings.TrimSpace(v) == "" {
				return fmt.Errorf("%s: empty selector", cl.Name)
			}
		}
		for _, m := range cl.Models {
			if strings.TrimSpace(m) == "" {
				return fmt.Errorf("%s: empty model", cl.Name)
			}
		}
	}
	return nil
}
func finite(v float64) bool { return !math.IsNaN(v) && !math.IsInf(v, 0) }

type Request struct {
	Model       string `json:"model"`
	InputTokens *int   `json:"inputTokens,omitempty"`
}
type Candidate struct {
	Name             string            `json:"name"`
	Labels           map[string]string `json:"labels"`
	Waiting          *int              `json:"waiting"`
	Running          *int              `json:"running"`
	MetricAgeSeconds *float64          `json:"metricAgeSeconds"`
}
type Decision struct {
	Name        string   `json:"name"`
	Class       string   `json:"class,omitempty"`
	Eligible    bool     `json:"eligible"`
	Reason      string   `json:"reason"`
	Score       float64  `json:"score"`
	LoadSeconds *float64 `json:"loadSeconds,omitempty"`
}

// Evaluate rejects unknown/ambiguous classes and stale/missing load data.
// This is a queue-pressure heuristic, not a latency prediction or SLO guarantee.
func (c Config) Evaluate(req Request, ep Candidate) Decision {
	d := Decision{Name: ep.Name, Reason: "unknown-class"}
	var chosen *Class
	for i := range c.Classes {
		cl := &c.Classes[i]
		match := true
		for k, v := range cl.Selector {
			if ep.Labels[k] != v {
				match = false
				break
			}
		}
		if match {
			if chosen != nil {
				d.Reason = "ambiguous-class"
				return d
			}
			chosen = cl
		}
	}
	if chosen == nil {
		return d
	}
	d.Class = chosen.Name
	allowed := false
	for _, m := range chosen.Models {
		if m == req.Model {
			allowed = true
			break
		}
	}
	if !allowed {
		d.Reason = "model-mismatch"
		return d
	}
	if req.InputTokens != nil && *req.InputTokens < 0 {
		d.Reason = "invalid-input-length"
		return d
	}
	if chosen.MaxInputTokens > 0 {
		if req.InputTokens == nil {
			d.Reason = "unknown-input-length"
			return d
		}
		if *req.InputTokens > chosen.MaxInputTokens {
			d.Reason = "input-too-long"
			return d
		}
	}
	if ep.Waiting == nil || ep.Running == nil || ep.MetricAgeSeconds == nil {
		d.Reason = "missing-metrics"
		return d
	}
	if *ep.Waiting < 0 || *ep.Running < 0 || !finite(*ep.MetricAgeSeconds) || *ep.MetricAgeSeconds < 0 {
		d.Reason = "invalid-metrics"
		return d
	}
	if *ep.MetricAgeSeconds > c.MaxMetricAgeSeconds {
		d.Reason = "stale-metrics"
		return d
	}
	load := (float64(*ep.Waiting) + float64(*ep.Running)) / chosen.Capacity
	d.Eligible = true
	d.Reason = "eligible"
	d.LoadSeconds = &load
	d.Score = 1 / (1 + load)
	return d
}
