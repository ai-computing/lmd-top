"""Execute the real recipe against a recording SDK, without hardware or downloads."""
import os
import contextlib
import io
from pathlib import Path
import runpy
import types
import unittest
from unittest.mock import Mock, patch

RECIPE = Path(__file__).resolve().parents[1] / 'assets/recipes/rbln-compile.py'


class RecipeTests(unittest.TestCase):
    def execute(self, quant, error=None):
        sdk = Mock()
        if error:
            sdk.from_pretrained.side_effect = error
        optimum = types.ModuleType('optimum')
        rbln = types.ModuleType('optimum.rbln')
        rbln.RBLNAutoModelForCausalLM = sdk
        publisher = Mock()
        self.sdk, self.publisher = sdk, publisher
        spec = Mock()
        with patch.dict(os.environ, {
            'MODEL_ID': 'org/quantized', 'OUTPUT': '/unused/output',
            'RBLN_QUANTIZATION': quant,
        }, clear=True), patch.dict('sys.modules', {'optimum': optimum, 'optimum.rbln': rbln}), \
                patch('importlib.metadata.version', return_value='test'), \
                patch('importlib.metadata.requires', return_value=[]), \
                patch('importlib.util.spec_from_file_location', return_value=spec), \
                patch('importlib.util.module_from_spec', return_value=publisher), \
                patch('os.listdir', return_value=[]):
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                runpy.run_path(str(RECIPE))
        return sdk, publisher

    def test_quantization_reaches_sdk_and_provenance(self):
        for quant, weights, activations in [('w8a8', 'int8', 'int8'), ('w4a16', 'int4', 'fp16')]:
            with self.subTest(quant=quant):
                sdk, publisher = self.execute(quant)
                kwargs = sdk.from_pretrained.call_args.kwargs
                self.assertEqual(kwargs['rbln_quantization'], {
                    'format': 'rbln', 'weights': weights, 'activations': activations,
                })
                self.assertFalse(kwargs['rbln_create_runtimes'])
                effective = publisher.publish.call_args.kwargs['effective_options']
                self.assertEqual(effective['rbln_quantization'], kwargs['rbln_quantization'])

    def test_none_leaves_sdk_quantization_unspecified(self):
        sdk, publisher = self.execute('none')
        self.assertNotIn('rbln_quantization', sdk.from_pretrained.call_args.kwargs)
        self.assertNotIn('rbln_quantization', publisher.publish.call_args.kwargs['effective_options'])

    def test_unknown_quantization_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'Unsupported RBLN_QUANTIZATION'):
            self.execute('typo')
        self.sdk.from_pretrained.assert_not_called()
        self.publisher.publish.assert_not_called()

    def test_sdk_quantization_rejection_is_not_masked(self):
        error = ValueError('Unexpected arguments: quantization')
        with self.assertRaisesRegex(ValueError, 'quantization'):
            self.execute('w8a8', error)
        self.sdk.from_pretrained.assert_called_once()
        self.publisher.publish.assert_not_called()


if __name__ == '__main__':
    unittest.main()
