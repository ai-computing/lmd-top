package heterogeneous

import (
	"context"
	"github.com/go-logr/logr"
	"github.com/llm-d/llm-d-router/pkg/epp/config/loader"
	"github.com/llm-d/llm-d-router/pkg/epp/datalayer"
	fwkdl "github.com/llm-d/llm-d-router/pkg/epp/framework/interface/datalayer"
	fwkplugin "github.com/llm-d/llm-d-router/pkg/epp/framework/interface/plugin"
	fwksched "github.com/llm-d/llm-d-router/pkg/epp/framework/interface/scheduling"
	extractormetrics "github.com/llm-d/llm-d-router/pkg/epp/framework/plugins/datalayer/extractor/metrics"
	sourcemetrics "github.com/llm-d/llm-d-router/pkg/epp/framework/plugins/datalayer/source/metrics"
	globalstrict "github.com/llm-d/llm-d-router/pkg/epp/framework/plugins/flowcontrol/fairness/globalstrict"
	fcfs "github.com/llm-d/llm-d-router/pkg/epp/framework/plugins/flowcontrol/ordering/fcfs"
	utilization "github.com/llm-d/llm-d-router/pkg/epp/framework/plugins/flowcontrol/saturationdetector/utilization"
	usagelimits "github.com/llm-d/llm-d-router/pkg/epp/framework/plugins/flowcontrol/usagelimits"
	anthropic "github.com/llm-d/llm-d-router/pkg/epp/framework/plugins/requesthandling/parsers/anthropic"
	openai "github.com/llm-d/llm-d-router/pkg/epp/framework/plugins/requesthandling/parsers/openai"
	vllmhttp "github.com/llm-d/llm-d-router/pkg/epp/framework/plugins/requesthandling/parsers/vllmhttp"
	maxscore "github.com/llm-d/llm-d-router/pkg/epp/framework/plugins/scheduling/picker/maxscore"
	single "github.com/llm-d/llm-d-router/pkg/epp/framework/plugins/scheduling/profilehandler/single"
	"github.com/llm-d/llm-d-router/pkg/epp/scheduling"
	"k8s.io/apimachinery/pkg/types"
	"os"
	"testing"
	"time"
)

func TestRenderedConfigLoadsInPinnedFramework(t *testing.T) {
	fwkplugin.Register(sourcemetrics.MetricsDataSourceType, sourcemetrics.MetricsDataSourceFactory)
	fwkplugin.Register(extractormetrics.MetricsExtractorType, extractormetrics.CoreMetricsExtractorFactory)
	fwkplugin.Register(globalstrict.GlobalStrictFairnessPolicyType, globalstrict.GlobalStrictFairnessPolicyFactory)
	fwkplugin.Register(fcfs.FCFSOrderingPolicyType, fcfs.FCFSOrderingPolicyFactory)
	fwkplugin.Register(usagelimits.StaticUsageLimitPolicyType, usagelimits.StaticPolicyFactory)
	fwkplugin.Register(maxscore.MaxScorePickerType, maxscore.MaxScorePickerFactory)
	fwkplugin.Register(single.SingleProfileHandlerType, single.SingleProfileHandlerFactory)
	fwkplugin.Register(anthropic.AnthropicParserType, anthropic.AnthropicParserPluginFactory)
	fwkplugin.Register(openai.OpenAIParserType, openai.OpenAIParserPluginFactory)
	fwkplugin.Register(vllmhttp.VllmHTTPParserType, vllmhttp.VllmHTTPParserPluginFactory)
	fwkplugin.Register(utilization.UtilizationDetectorType, utilization.UtilizationDetectorFactory)
	fwkplugin.Register(PluginType, Factory)
	fwkplugin.Register(ExtractorType, ExtractorFactory)
	text, err := os.ReadFile("../../examples/rendered-epp.yaml")
	if err != nil {
		t.Fatal(err)
	}
	logger := logr.Discard()
	raw, _, err := loader.LoadRawConfig(text, logger)
	if err != nil {
		t.Fatal(err)
	}
	handle := fwkplugin.NewEppHandle(context.Background(), func() []types.NamespacedName { return nil })
	cfg, err := loader.InstantiateAndConfigure(raw, handle, logger)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = datalayer.ValidateAndOrderDataDependencies(handle.GetAllPlugins()); err != nil {
		t.Fatal(err)
	}
	runtime := datalayer.NewRuntime(time.Second)
	if err = runtime.Configure(cfg.DataConfig, logger); err != nil {
		t.Fatal(err)
	}
	// The weighted wrapper must preserve the Filter interface, not merely assign zero score.
	ep := fwksched.NewEndpoint(&fwkdl.EndpointMetadata{PodName: "unknown", Labels: map[string]string{}}, fwkdl.NewMetrics(), nil)
	scheduler := scheduling.NewSchedulerWithConfig(cfg.SchedulerConfig)
	if _, err = scheduler.Schedule(context.Background(), &fwksched.InferenceRequest{TargetModel: "demo-model"}, []fwksched.Endpoint{ep}); err == nil {
		t.Fatal("unknown endpoint was routed")
	}
}
