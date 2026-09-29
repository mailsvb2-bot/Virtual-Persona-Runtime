export const mediaRuntime = () => window.__vprMediaRuntime;
export const runtimeFetch = window.fetch.bind(window);
export const runtimeMediaDevices = () => mediaRuntime()?.mediaDevices ?? navigator.mediaDevices;
export const createRuntimeAudioContext = (options) => {
    const Constructor = (mediaRuntime()?.AudioContext ?? window.AudioContext);
    return new Constructor(options);
};
export const createRuntimeAudioWorkletNode = (context, name) => {
    const Constructor = (mediaRuntime()?.AudioWorkletNode ?? window.AudioWorkletNode);
    return new Constructor(context, name);
};
export const createRuntimeMediaStream = (tracks) => {
    const Constructor = (mediaRuntime()?.MediaStream ?? window.MediaStream);
    return tracks ? new Constructor(tracks) : new Constructor();
};
export const createRuntimePeerConnection = (configuration) => {
    const Constructor = (mediaRuntime()?.RTCPeerConnection ?? window.RTCPeerConnection);
    return new Constructor(configuration);
};
export const setMediaSrcObject = (element, value) => {
    const setter = mediaRuntime()?.setSrcObject;
    if (setter)
        setter(element, value);
    else
        element.srcObject = value;
};
export const requestVideoFrame = (element, callback) => {
    const request = mediaRuntime()?.requestVideoFrame;
    if (request) {
        request(element, callback);
        return true;
    }
    const nativeRequest = element.requestVideoFrameCallback;
    if (typeof nativeRequest !== "function")
        return false;
    nativeRequest.call(element, callback);
    return true;
};
