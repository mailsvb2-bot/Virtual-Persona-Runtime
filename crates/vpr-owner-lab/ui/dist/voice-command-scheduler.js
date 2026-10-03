export class PlaybackAwareCommandScheduler {
    send;
    currentPlayback = null;
    generation = 0;
    pendingCommands = 0;
    queueTail = Promise.resolve();
    constructor(send) {
        this.send = send;
    }
    get hasActivePlayback() {
        return this.currentPlayback !== null;
    }
    get hasPendingPlayback() {
        return this.pendingCommands > 0;
    }
    dispatch(command) {
        this.pendingCommands += 1;
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
                if (generation === this.generation && this.pendingCommands > 0) {
                    this.pendingCommands -= 1;
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
        if (this.pendingCommands > 0)
            this.pendingCommands -= 1;
        barrier.resolve();
    }
    interrupt() {
        this.generation += 1;
        this.pendingCommands = 0;
        const barrier = this.currentPlayback;
        this.currentPlayback = null;
        barrier?.resolve();
    }
}
