class VprMicCaptureProcessor extends AudioWorkletProcessor {
  process(inputs) {
    const channel = inputs[0]?.[0];
    if (channel && channel.length > 0) {
      const copy = new Float32Array(channel);
      this.port.postMessage(copy.buffer, [copy.buffer]);
    }
    return true;
  }
}

registerProcessor("vpr-mic-capture", VprMicCaptureProcessor);
