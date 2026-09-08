export function copyOnClick(button, text) {
    const label = button.textContent;
    button.setAttribute('aria-live', 'polite');
    let reset;
    button.addEventListener('click', async () => {
        try {
            await navigator.clipboard.writeText(text);
            button.textContent = 'Copied!';
        } catch {
            button.textContent = 'Copy failed';
        }
        clearTimeout(reset);
        reset = setTimeout(() => { button.textContent = label; }, 2000);
    });
}
