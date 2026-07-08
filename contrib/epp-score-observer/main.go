// Command epp-score-observer is a drop-in replacement for the llm-d endpoint-picker
// (EPP) binary that additionally registers the endpoint-score-observer Picker plugin.
//
// It reuses the upstream runner verbatim (github.com/llm-d/llm-d-router/cmd/epp/runner),
// so all standard flags/plugins behave identically — the only difference is that our
// custom plugin type becomes available for the config file to reference as the picker.
//
// This avoids forking the endpoint-picker: we just register one more factory into the
// global plugin registry before the runner loads the config.
package main

import (
	"os"

	ctrl "sigs.k8s.io/controller-runtime"

	"github.com/llm-d/llm-d-router/cmd/epp/runner"
	fwkplugin "github.com/llm-d/llm-d-router/pkg/epp/framework/interface/plugin"

	"github.com/etri/epp-score-observer/pkg/scoreobserver"
)

func main() {
	// Register our picker so the config file can reference type: endpoint-score-observer.
	// The runner's own registerAllPlugins() populates the same global registry; order is
	// irrelevant as long as both run before config parsing (they do — this is before Run).
	fwkplugin.Register(scoreobserver.PluginType, scoreobserver.Factory)

	ctx := ctrl.SetupSignalHandler()
	if err := runner.NewRunner().Run(ctx); err != nil {
		os.Exit(1)
	}
}
