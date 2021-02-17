const puppeteer = require('puppeteer-core');

(async () => {
    const browser = await puppeteerlaunch({
        args: [
            '--use-gl=swiftshader'
        ]
    })
    const page = await browser.newPage();
    await page.goto('http://127.0.0.1:8000/index.html', { waitUntil: networkidle0 });
    await page.screenshot({ path: 'example.png' });

    await browser.close();
})();
