export class PlaybackAwareCommandScheduler {
    send;
    currentPlayback = null;
    generation = 0;
    queueTail = Promise.resolve();
    constructor(send) {
        this.send = send;
    }
    get hasActivePlayback() {
        return this.currentPlayback !== null;
    }
    dispatch(command) {
        const generation = this.generation;
        let resolveResult;
        let rejectResult;
        const result = new Promise((resolve, reject) => {
            resolveResult = resolve;
            rejectResult = reject;
        });
        const run = async () => {
            if (generation !== this.generation) {
                resolveResult(false);
                return;
            }
            let release;
            const barrier = {
                promise: new Promise((done) => {
                    release = done;
                }),
                resolve: () => release(),
            };
            this.currentPlayback = barrier;
            try {
                await this.send(command);
                if (generation !== this.generation) {
                    if (this.currentPlayback === barrier) {
                        this.currentPlayback = null;
                        barrier.resolve();
                    }
                    resolveResult(false);
                    return;
                }
                resolveResult(true);
                await barrier.promise;
            }
            catch (error) {
                if (this.currentPlayback === barrier) {
                    this.currentPlayback = null;
                    barrier.resolve();
                }
                rejectResult(error);
            }
        };
        const scheduled = this.queueTail.then(run, run);
        this.queueTail = scheduled.then(() => undefined, () => undefined);
        return result;
    }
    playbackDone() {
        const barrier = this.currentPlayback;
        if (!barrier)
            return;
        this.currentPlayback = null;
        barrier.resolve();
    }
    interrupt() {
        this.generation += 1;
        const barrier = this.currentPlayback;
        this.currentPlayback = null;
        barrier?.resolve();
    }
}
