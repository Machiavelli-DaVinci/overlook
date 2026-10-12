"""Apply the Overlook identity to a copy of the earlier design prototype."""
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'source/mail-prototype.html'
text = SOURCE.read_text()
text = re.sub(r"@import url\([^\n]+\);", "@font-face{font-family:Inter;src:url('fonts/InterVariable.woff2');font-weight:100 900}\n@font-face{font-family:'Source Serif 4';src:url('fonts/SourceSerif4Display-Regular.woff2')}", text)
replacements = {
    '#f5f6f9':'#F4F3EE', '#17191e':'#101A21', '#202228':'#17242C',
    '#f3f5f8':'#F0F1EC', '#1b1d23':'#121F27', '#202633':'#192B35', '#eceef4':'#F0EBDD',
    '#6d7687':'#5E6D73', '#9bA2b1':'#A8B7BA', '#e8ebf1':'#DCE2DF', '#333640':'#30424B',
    '#eef1f7':'#E8EDE9', '#2b2e37':'#23353F', '#2864e9':'#365F73', '#91b1ff':'#A9CDD9',
    '#3a70ef':'#A9CDD9', '#eaf0ff':'#E3EDEA', '#283651':'#28404C',
    '#eee6ff':'#E4EBE3', '#47395e':'#34483F', '#7d57b3':'#42614E', '#d6b5ff':'#C2D5C3',
    '#e3eafa':'#E3EDEA','#35425e':'#28404C','#465f9b':'#365F73','#c8d7ff':'#A9CDD9',
    '#dfeaff':'#E0EAF0','#31466c':'#2D414E','#3e6dbc':'#365F73','#b8d1ff':'#A9CDD9',
    '#e5eafb':'#E3EDEA','#374361':'#28404C','#536eb0':'#365F73','#bccdfd':'#A9CDD9',
    '#fff,#17242C':'#FFFEFB,#17242C', '--om-on:#fff':'--om-on:light-dark(#fff,#10232D)',
}
for old,new in replacements.items():
    text=text.replace(old,new)
text = text.replace('<span class="om-brandmark" aria-hidden="true">O</span>Overlook', '<img class="ol-product-icon" src="assets/overlook-app-icon.svg" alt=""><span>Overlook</span>')
text=text.replace("appearance:'system'", "appearance:new URLSearchParams(location.search).get('theme')==='dark'?'dark':'light'")
text=text.replace("direction:'Refined mail client'", "direction:'Overlook brand preview'")
text=text.replace("d.direction!=='Refined mail client'", "d.direction!=='Overlook brand preview'")
text=text.replace("root.dataset.reading=design.reading;", "root.dataset.reading=design.reading;if(parent!==window)parent.postMessage({type:'overlook-preview-theme',theme:design.appearance},'*');")
text=text.replace("restoreState(window.openai?.widgetState);", "window.addEventListener('message',event=>{if(event.source===parent&&event.data?.type==='overlook-appearance'&&['light','dark'].includes(event.data.theme)){design.appearance=event.data.theme;applyDesign();icons();}});\n restoreState(window.openai?.widgetState);")
text=text.replace("if(globalThis.Tweak)", "if(parent!==window)new ResizeObserver(()=>parent.postMessage({type:'overlook-preview-height',height:Math.ceil(root.getBoundingClientRect().height)},'*')).observe(root);\n if(globalThis.Tweak)")
text=text.replace('All changes saved','Up to date')
extra = '''<style>
html,body{margin:0;padding:0;background:transparent;color-scheme:light dark}
#overlook-refined .om-brand{font-family:'Source Serif 4',Georgia,serif;font-size:26px;font-weight:400;letter-spacing:-.6px}
#overlook-refined .om-brand .ol-product-icon{width:32px;height:32px}
#overlook-refined .om-header-actions{padding-right:0}
#overlook-refined .om-layout{min-height:700px}
#overlook-refined :focus-visible{outline:2px solid light-dark(#986020,#FFBA6B);outline-offset:3px}
#overlook-refined .om-search{border-color:light-dark(#74868B,#71878D)}
#overlook-refined .om-doc-stripe{background:linear-gradient(135deg,#365F73,#536E63)!important}
button{cursor:pointer}
</style>'''
text=text.replace('</style>', '</style>'+extra,1)
result='<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Overlook mail · Brand application preview</title><script src="vendor/lucide.min.js"></script></head><body>'+text+'</body></html>'
(ROOT/'mail-preview.html').write_text(result)
print('Saved standalone mail brand application preview')
