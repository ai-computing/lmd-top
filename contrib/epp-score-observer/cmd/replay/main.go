// replay runs exactly the same Evaluate function used by the EPP adapter, offline.
package main

import (
	"encoding/json"
	"fmt"
	"github.com/etri/epp-score-observer/pkg/heterogeneous"
	"io"
	"os"
)

func read(path string, v any) error {
	f, err := os.Open(path)
	if err != nil {
		return err
	}
	defer f.Close()
	d := json.NewDecoder(f)
	d.DisallowUnknownFields()
	if err = d.Decode(v); err != nil {
		return err
	}
	if err = d.Decode(new(any)); err != io.EOF {
		return fmt.Errorf("expected one JSON value")
	}
	return nil
}
func run() error {
	if len(os.Args) != 3 {
		return fmt.Errorf("usage: replay parameters.json requests.json")
	}
	var cfg heterogeneous.Config
	if err := read(os.Args[1], &cfg); err != nil {
		return err
	}
	if err := cfg.Validate(); err != nil {
		return err
	}
	var cases []struct {
		Request    heterogeneous.Request     `json:"request"`
		Candidates []heterogeneous.Candidate `json:"candidates"`
	}
	if err := read(os.Args[2], &cases); err != nil {
		return err
	}
	out := json.NewEncoder(os.Stdout)
	out.SetIndent("", "  ")
	for _, c := range cases {
		decisions := make([]heterogeneous.Decision, 0, len(c.Candidates))
		best := -1.0
		winners := []string{}
		for _, ep := range c.Candidates {
			d := cfg.Evaluate(c.Request, ep)
			decisions = append(decisions, d)
			if d.Eligible {
				if d.Score > best {
					best = d.Score
					winners = []string{d.Name}
				} else if d.Score == best {
					winners = append(winners, d.Name)
				}
			}
		}
		if err := out.Encode(map[string]any{"request": c.Request, "decisions": decisions, "maxScoreCandidates": winners}); err != nil {
			return err
		}
	}
	return nil
}
func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
