import { copyOnClick } from './copy-button.js';

const entry = new URL('./console-runner.js', import.meta.url).href;
copyOnClick(document.getElementById('copy-console-script'),
    `void import(${JSON.stringify(entry)}).then(({ run }) => run()).catch(console.error);`);
