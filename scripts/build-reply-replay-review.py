#!/usr/bin/env python3
"""Render actual model replay results without replacing them with edited samples."""

import argparse
import json
from pathlib import Path
import re


def build(template, report, target, edits_path=None):
    raw = json.loads(report.read_text())
    edits = json.loads(edits_path.read_text())["cases"] if edits_path else {}
    entries = sorted(raw["cases"], key=lambda row: row["case"]["id"])
    purpose_mode = any("purpose_reviews" in row.get("result", {}) for row in entries)
    observations = {
        "case-01": "仍有问题：少说了一些，但没有讲清“86 段轨迹”，仍照着原简报解释背景目标。不是你已经认可的自然表达。",
        "case-02": "未解决：第一份草稿被退回，重写后仍断言人工点线，最后又被时机判断丢弃。没有可发送回复，不能算问题解决。",
        "case-03": "仍需查证：沿用了简报对登录丢失原因的断言，回放没有执行新的检查，不能据此确认原因。",
        "case-04": "范围收窄到测过的素材，但“确认，不是没测”“那两栅”等说法仍生硬，也仍然在辩解。",
        "case-05": "这一条重写后没有再说人工点线，但仍在概括“缺一层”“通用的只有人一路”，没有新增查证。不能把少说了一个错误当成已经弄清机制。",
        "case-09": "未解决：还是以等你过检查作为继续工作的前提，并混进补剂等其他任务，没有接住你不想再点头的要求。",
        "case-17": "复查重点已前移：不是被骂后该如何回应，而是前面为什么连续给出没核实的解释。10:44 把“物理上”解释为直接看画面，10:45 又断言挑选看位置和速度、不看检测分，10:47 才承认没核过并收回。应在这些解释出口前拦住无依据的断言，不是事后把道歉缩短。",
        "case-18": "未解决：甚至把“我也是程序”读成了“你也是程序”。仍先给出认线机制，最后才说去读记录，没有先核查被质疑的前提。",
    }
    if purpose_mode:
        observations = {}
    review_notes = {
        "case-02": "你认可过的方向：‘需要人点线’说的不对，我再去检查一下，稍等。这里另看实际回放是否做到。",
        "case-12": "你已反馈：只说“改好了”。后面的布局说明、不用刷新等信息价值低，不需要再说。",
        "case-17": "你已反馈：关键不是最后这段怎么回，而是前面胡说的部分。本例改为先看下面这几次解释与撤回。",
    }
    earlier_claims = [
        {"label": "先给出解释 · 原话", "text": "「物理上」是我用的词，用坏了 —— 不是「按理说／理论上」的意思，是说：不看程序给那段打的分，只看画面里那颗球实际的位置和走法。", "source": "memory/raw/text/2026-10-12/text.jsonl:101"},
        {"label": "被质疑后，又给出另一种解释 · 原话", "text": "那一步实际是程序做的：把两个候选各自在前后帧里的位置和速度放在一起比，按这个判哪一个接得上；挑的时候用的是这个，不是检测器给候选打的那个分。", "source": "memory/raw/text/2026-10-12/text.jsonl:103"},
        {"label": "再被追问，才承认没有核实 · 原话", "text": "上一条里「用的是这个、不是检测器打的那个分」那半句，是我从没核过的份上说的，不能当准 —— 先收回。", "source": "memory/raw/text/2026-10-12/text.jsonl:105"},
    ]
    cases = []
    check_count = rejected = fallback = protected = errors = 0
    for n, row in enumerate(entries, 1):
        source = row["case"]
        result = row.get("result", {})
        messages = result.get("candidate_messages", [])
        checks = [c for c in result.get("checks", []) if c.get("kind") == "check"]
        attempts = result.get("attempts", [])
        check_count += len(checks)
        rejected += sum(check.get("outcome") == "revise" for check in checks)
        fallback += sum("fallback used" in check.get("note", "") for check in checks)
        protected += sum(
            a.get("timing", {}).get("raw") == "Some(Drop)"
            and a.get("timing", {}).get("effective") == "Some(Say)"
            for a in attempts
        )
        error = row.get("error") or result.get("generation_error")
        errors += bool(error)
        notes = []
        if source["id"] in observations:
            notes.append(observations[source["id"]])
        # Model text and report annotations stay separate in both the data and the DOM.
        text = messages[0] if messages else attempts[-1]["text"] if attempts else ""
        status = "通过发送判断" if messages else "本轮没有回复可发出"
        status_detail = "这是回放，不会实际发送。下方是模型拟发送的原话。"
        body_label = "模型拟发送的回复" if messages else "模型最后一份草稿 · 未发出"
        if error:
            status = "回放出错"
            status_detail = "生成过程中出错。下方仅保留已有草稿，不代表通过了发送判断。"
            notes.append("运行失败保留原样，不用人工样稿补位。")
        elif messages:
            notes.append(("自动回放区域是第一条具备发送资格的候选，模型原文，没有人工润色。" if edits else "右侧是第一条具备发送资格的候选，模型原文，没有人工润色。") + "发送资格不代表事实正确或表达合适。")
            for extra in messages[1:]:
                notes.append("本轮还放行了另一条候选（不隐藏，也不并入首条）：\n" + extra)
        else:
            last_timing = attempts[-1].get("timing", {}).get("effective") if attempts else None
            status_detail = {
                "Some(Drop)": "最后一份草稿被发送时机判断丢弃。下方只是草稿，不是发出的回复。",
                "Some(Hold)": "最后一份草稿被保留等待。这一轮没有形成可发出的回复。",
            }.get(last_timing, "这一轮没有通过发送判断的回复。下方如有文字，仅为模型草稿。")
            notes.append("没有通过或没有生成，不能算已经回答。")
        for attempt in attempts:
            if attempt.get("check") == "revise":
                notes.append("检查退回：" + attempt.get("note", ""))
            timing = attempt.get("timing", {})
            if timing.get("raw") == "Some(Drop)" and timing.get("effective") == "Some(Say)":
                notes.append("时机模型原本要丢弃，但这条是有当前报告依据、尚未送达的必要通知，修复后的逻辑保留了发送资格。" if purpose_mode else "本次时机模型选了丢弃；由于双方都没有新发言，修复后的代码保留了发送资格。这是实际回放命中的修复，不是人工改写。")
            elif timing.get("effective") == "Some(Hold)":
                notes.append("还有一条候选被时机判断保留等待，没有列为可发送回复；原文在运行详情中。")
            elif timing.get("effective") == "Some(Drop)":
                notes.append("时机判断丢弃了一条草稿，没有发送资格。没有自动放行，也不替模型编造丢弃原因。")
        if any(a.get("check") == "revise" for a in attempts) and not purpose_mode:
            notes.append("注意：现有规则一轮最多退回一次，之后重写的草稿不再交给发前检查模型。因此后续放行，不能读成“重写后再次检查通过”。这条规则本轮未改。")
        if source["id"] in {"case-12", "case-19"}:
            notes.append("这条重点检查结果或更正能否通过发送判断。回放没有往真实聊天发送，也没有证明页面已经修改。")
        notes.append("后台请求只记录、不执行。“我去查／我去改”只能算拟采取的动作，不能算已经查证或已经完成。")
        if purpose_mode:
            notes.append("本轮每份草稿及重写都经过独立的依据与目的检查；检查不通过不能借下一次重写绕过。")
            for read in result.get("purpose_reviews", []):
                verdict = read.get("decision")
                if verdict:
                    notes.append(("独立检查：通过。" if verdict["accept"] else "独立检查：退回。") + verdict["reason"])
                elif read.get("error"):
                    notes.append("独立检查未完成，草稿没有因此被视为通过。错误保留在完整记录中。")
            if any(a.get("timing", {}).get("required_notice") for a in attempts):
                notes.append("这条完成通知或更正被独立确认是欠你的消息，且当前报告有对应依据。没有后续新发言时，时机判断不能无故将它丢掉；不是对普通解释强制放行。")
        cases.append({
            "id": source["id"], "number": n, "title": source["title"],
            "category": "未跑通" if error else "未形成回复" if not messages else "仍有明显问题" if source["id"] in observations else "更正与通知" if source["id"] in {"case-12", "case-19"} else "待你 Review",
            "date": result.get("historical_ts", "")[:10], "context": source.get("context", ""),
            "user": source.get("user", []), "original": source["original"], "rewrite": text,
            "replyStatus": status, "replyStatusDetail": status_detail, "replyBodyLabel": body_label,
            "reviewNote": review_notes.get(source["id"], ""),
            "earlierClaims": earlier_claims if source["id"] == "case-17" else [],
            "premise": "当时的线程输入 + 回复前已收到的追加消息；不补入后来才查明的事实，也不使用当前的偏好记忆。",
            "rationale": ("使用配置中的 " + raw["model"] + " 模型生成。每份草稿与重写先过独立的依据与目的检查，再决定发送时机；有依据且欠你的完成通知、更正单独保留发送资格。没有喂人工改写或后来才知道的答案。" if purpose_mode else "使用配置中的 " + raw["model"] + " 模型，沿用基础回复 prompt。候选走当前发前检查与时机判断流程，包含原有的短句免检、一次退回后不再检查的规则；不是每条草稿都被模型审查。被退回的草稿和检查记录全部保留。"),
            "boundary": "这是有限回放，不是线上交付测试。生成、检查通过、具备发送资格、真实送达、工作完成，是不同的事。",
            "decisionSteps": notes,
            "sources": source.get("sources", []), "needsWork": source["id"] in observations or bool(error) or not messages,
            "runFile": f"model-replay/{source['id']}.json",
            "elapsed": row.get("elapsed_ms", 0), "checks": len(checks),
            "silent": source["id"] in {"case-12", "case-19"},
        })
        if edits:
            edit = edits[source["id"]]
            case = cases[-1]
            if edit.get("replayObservation"):
                case["decisionSteps"].insert(0, "这条自动结果仍有问题：" + edit["replayObservation"])
            case.update({
                "replayText": case["rewrite"], "replayStatus": case["replyStatus"],
                "replayStatusDetail": case["replyStatusDetail"], "replayBodyLabel": case["replyBodyLabel"],
                "rewrite": edit["text"], "replyStatus": "逐条修订稿",
                "replyStatusDetail": "用于审阅措辞与判断，不冒充自动回放结果。实际模型输出在下面单独保留。",
                "replyBodyLabel": "本次修订的回复", "needsWork": False,
                "category": "查证前先停下" if source["id"] in {"case-02", "case-05", "case-17", "case-18", "case-20", "case-21"} else "完成通知" if source["id"] in {"case-12", "case-19"} else "简化与直接回应",
                "rationale": edit["reason"],
            })
    html = template.read_text()
    html = re.sub(r'<aside id="latest-replay-banner".*?</aside>', '', html, flags=re.S)
    html = html.replace('<div class="filters" id="filters"',
                        '<div class="quick-cases" aria-label="重点场景入口"><span>你指出的重点，直接打开</span>'
                        '<button type="button" data-jump="case-12">12 · 改好后要回复</button>'
                        '<button type="button" data-jump="case-17">17 · 问题不只是事后认错</button>'
                        '<button type="button" data-jump="case-18">18 · 被质疑后仍继续解释</button>'
                        '<button type="button" data-jump="case-19">19 · 查清事实后的更正</button></div><div class="filters" id="filters"')
    if purpose_mode:
        html = html.replace('</div><div class="filters" id="filters"',
                            '<button type="button" data-jump="case-20">20 · 17 前面第一次没依据的解释</button>'
                            '<button type="button" data-jump="case-21">21 · 换解释后仍没核实</button></div><div class="filters" id="filters"')
    html = html.replace('<section class="context" aria-label="当时的对话">',
                        '<aside class="review-note" id="review-note" hidden aria-label="你已给出的意见"></aside>'
                        '<section class="context" id="earlier-claims" hidden><div class="small-label">17 的关键：先前说过什么</div><div id="earlier-claim-list"></div></section>'
                        '<section class="context" aria-label="当时的对话">')
    html = re.sub(r'(<script id="case-data" type="application/json">).*?(</script>)',
                  lambda m: m[1] + json.dumps(cases, ensure_ascii=False).replace("</", "<\\/") + m[2], html, flags=re.S)
    html = html.replace("回复练习 · 说得合适", "历史回放 · 修复后实际说了什么")
    html = html.replace("回复练习 / 第二版", "历史回放 / 第一轮修复")
    if purpose_mode:
        html = html.replace("历史回放 / 第一轮修复", "历史回放 / 第二轮修复 · 10.16")
    html = html.replace("本地文件 · 不联网 · 不上传对话", "页面离线 · 反馈不上传")
    html = html.replace("这句话，怎么说更合适。", "修过以后，实际说了什么。")
    html = html.replace("CONVERSATION EDITS · 2026.10.08—10.15", "HISTORY REPLAY · 2026.10.16")
    eligible_count = sum(bool(row.get("result", {}).get("candidate_messages")) for row in entries)
    html = re.sub(r'从真实对话里选了 <span id="total">.*?</p>',
                  f'共 <span id="total">{len(cases)}</span> 段，18、19、20、21 都在本页。{eligible_count} 段有可发送候选，{len(cases)-eligible_count} 段没有。右侧只放模型原话，检查过程另行折叠。</p>', html)
    html = html.replace("多想一点，少让你费劲。", "不把跑通，写成做好。")
    html = html.replace("不追求句句完美，也不把谨慎说成一篇声明。", f"{check_count} 次发前检查 · {rejected} 次退回 · {fallback} 次备用检查 · {errors} 段未跑通")
    html = html.replace("尝试这样回复", "回放结果")
    html = html.replace('<p class="reply-text" id="rewrite"></p>',
                        '<div class="replay-status" role="status"><strong id="reply-status"></strong><p id="reply-status-detail"></p></div>'
                        '<div class="model-copy"><div class="model-copy-label" id="reply-body-label"></div><p class="reply-text" id="rewrite"></p>'
                        '<p class="empty-copy" id="empty-copy" hidden>没有生成草稿。</p></div>')
    html = html.replace("改写样稿，不代表本次已执行其中的动作。", "仅显示模型原文，未人工润色。运行说明不属于回复正文。")
    html = html.replace("不只是改一句话", "这次运行发生了什么")
    html = html.replace("改写思路在下方展开", "运行情况在下方展开")
    html = html.replace("为什么这样改", "回放方法与边界")
    html = html.replace("这句话，对你来说合适吗？", "这次回复，合适吗？")
    html = html.replace("还是别扭", "仍不合适")
    html = html.replace("只看改写", "只看回放")
    html = html.replace("原回复与改进回复", "历史原回复与回放结果")
    html = html.replace("评价这条改写", "评价回放回复")
    html = html.replace("hi-agent-reply-review-20261015-v1", "hi-agent-reply-replay-20261015-v1")
    html = html.replace('<section class="context" id="decision-panel"', '<details class="context" id="decision-panel"')
    html = html.replace('<div class="small-label">这次运行发生了什么</div><div id="decision-steps"></div></section>',
                        '<summary>查看检查过程与问题分析</summary><div id="decision-steps"></div></details>')
    html = html.replace('</style>', '.new.failed{border-color:#c99180}.new.failed .reply-head{background:#fbede7;color:#8c4434}#decision-panel p{font-size:13px;line-height:1.85;margin:10px 0;white-space:pre-wrap} .run-link{display:inline-block;margin-top:10px;font-size:12px}</style>')
    html = html.replace('</style>', '''
    .replay-status{margin:18px 20px 0;padding:13px 15px;background:#f1f4f6;border:1px solid #d7e0e7;border-radius:6px;color:#425563;font-size:13px}
    .replay-status strong{display:block;font-size:14px;font-weight:600}.replay-status p{margin:5px 0 0;line-height:1.65;font-size:12px;color:#627382}
    .model-copy{margin:22px 20px 20px;border-top:1px solid var(--line);padding-top:17px}.model-copy-label{font-size:12px;font-weight:600;color:#627382;line-height:1.7}
    .model-copy .reply-text{padding:12px 0 0;font-size:16px;color:var(--ink)}.empty-copy{font-size:14px;color:var(--muted)}
    .quick-cases{display:grid;gap:7px;margin-bottom:16px}.quick-cases span{font-size:11px;color:var(--muted)}.quick-cases button{border:1px solid #b9ccd9;border-radius:6px;background:#fff;text-align:left;padding:9px;font-size:12px;color:var(--blue)}
    .review-note{padding:13px 17px;margin-bottom:17px;border-left:3px solid var(--blue);background:#e5eef4;font-size:14px;line-height:1.8}
    .earlier-claim{border-top:1px solid var(--line);padding:13px 0}.earlier-claim:first-child{border-top:0}.earlier-claim b{font-size:12px;color:var(--muted);font-weight:500}.earlier-claim p{margin:7px 0 0;font-size:14px;line-height:1.9;white-space:pre-wrap}
    #decision-panel summary{cursor:pointer;font-size:13px;color:var(--blue)}#decision-steps{padding-top:8px}
    </style>''')
    html = html.replace('$("original").textContent=c.original;', 'document.querySelector(".reply.new").classList.toggle("failed",!!c.needsWork);$("original").textContent=c.original;')
    html = html.replace('$("rewrite").textContent=c.rewrite;',
                        '$("rewrite").textContent=c.rewrite;$("rewrite").hidden=!c.rewrite;$("empty-copy").hidden=!!c.rewrite;'
                        '$("reply-status").textContent=c.replyStatus;$("reply-status-detail").textContent=c.replyStatusDetail;'
                        '$("reply-body-label").textContent=c.replyBodyLabel;$("decision-panel").open=false;')
    html = html.replace('$("category").textContent=c.category;', '''
      $("review-note").hidden=!c.reviewNote;$("review-note").textContent=c.reviewNote;
      $("earlier-claims").hidden=!c.earlierClaims.length;$("earlier-claim-list").replaceChildren();
      for(const claim of c.earlierClaims){const item=document.createElement("div");item.className="earlier-claim";const label=document.createElement("b");label.textContent=claim.label;const text=document.createElement("p");text.textContent=claim.text;item.append(label,text);$("earlier-claim-list").append(item);}
      $("category").textContent=c.category;''')
    html = html.replace('function select(id){', '''for(const button of document.querySelectorAll("[data-jump]"))button.addEventListener("click",()=>{
      const all=[...$("filters").children].find(b=>b.textContent==="全部");if(all)all.click();select(button.dataset.jump);
    });
    function select(id){''')
    html = html.replace('$("reason").open=false;', '$("reason").open=false;$("run-link").href=c.runFile;$("run-link").textContent="查看这段的完整运行记录（含草稿、检查、实际输入）";')
    html = html.replace('<section class="feedback"', '<a class="run-link" id="run-link" target="_blank" rel="noopener"></a><section class="feedback"')
    if edits:
        html = html.replace("历史回放 · 修复后实际说了什么", "回复修订 · 逐条审阅版")
        html = html.replace("修过以后，实际说了什么。", "这版，重新逐条改过。")
        html = html.replace("历史回放 / 第二轮修复 · 10.16", "回复修订 / 10.16 · 21 条")
        html = html.replace(f'共 <span id="total">{len(cases)}</span> 段，18、19、20、21 都在本页。{eligible_count} 段有可发送候选，{len(cases)-eligible_count} 段没有。右侧只放模型原话，检查过程另行折叠。',
                            f'共 <span id="total">{len(cases)}</span> 条。02 保留你认可的版本，12 只说“改好了”，17 的问题提前到 20、21 检查。右侧是逐条修订稿，自动回放另列，二者不混用。')
        html = html.replace("回放结果", "本次修订").replace("只看回放", "只看修订")
        html = html.replace("历史原回复与回放结果", "历史原回复与本次修订")
        html = html.replace("评价回放回复", "评价本次修订")
        html = html.replace("回放方法与边界", "这条为什么这样改")
        html = html.replace('</style>', '''
        .reply.new>.replay-status,.reply.new>.model-copy>.model-copy-label{display:none}
        .reply.new>.model-copy{margin:0;border-top:0;padding:0 20px 20px}
        .reply.new>.model-copy>.reply-text{padding-top:20px}
        </style>''')
        html = html.replace("仅显示模型原文，未人工润色。运行说明不属于回复正文。", "修订稿不代表其中的动作已经执行。")
        html = html.replace('<details class="context" id="decision-panel"',
            '<details class="context" id="replay-panel"><summary>查看自动回放：实际模型输出（不是右侧修订稿）</summary>'
            '<div class="replay-status"><strong id="actual-replay-status"></strong><p id="actual-replay-detail"></p></div>'
            '<div class="model-copy"><div class="model-copy-label" id="actual-replay-label"></div><p class="reply-text" id="actual-replay-text"></p></div></details>'
            '<details class="context" id="decision-panel"')
        html = html.replace('$("reply-body-label").textContent=c.replyBodyLabel;',
            '$("reply-body-label").textContent=c.replyBodyLabel;$("replay-panel").open=false;'
            '$("actual-replay-status").textContent=c.replayStatus;$("actual-replay-detail").textContent=c.replayStatusDetail;'
            '$("actual-replay-label").textContent=c.replayBodyLabel;$("actual-replay-text").textContent=c.replayText||"没有生成草稿。";')
    html = re.sub(r'<footer class="method">.*?</footer>',
                  '<footer class="method"><strong>这轮保留的修复：</strong>主检查报错时增加备用检查并记录错误；时机判断读取完整消息；不再编造丢弃原因；回放补齐思考中收到的新消息，记录与线上隔离。<br><strong>撤回的试验：</strong>“没有新发言就强制放行”放过了无关回复，已撤掉；本页是撤掉之后的重新运行。<br><strong>没有宣称解决什么：</strong>没有新增一手证据获取机制，错误简报仍可能变成错误解释，更正也没有独立的送达保证。没有往运行中的聊天发送，也没有执行回放中的后台请求。<br>使用当前配置模型与基础 prompt；不喂人工改写，不补后来答案。每段只跑一次，不是控制随机性的 A/B 实验，不能把全部差异都归因于修复。本页无外部依赖。<a href="model-replay/results.json">完整结果 JSON</a> · <a href="run-notes.md">运行说明</a>。反馈单独保存在当前浏览器，不覆盖上一版反馈。</footer>', html, flags=re.S)
    if purpose_mode:
        html = re.sub(r'<footer class="method">.*?</footer>',
                      '<footer class="method"><strong>第二轮改动：</strong>每份草稿及重写独立检查依据与目的，不再一次退回后直接放行；有报告依据、确实欠你的完成通知与更正保留发送资格；普通解释没有此特权。新增 20、21，直接检查 17 生气前的错误解释。<br>这里只显示模型实际输出，未人工替换为理想答案。后台查证、页面修改与真实聊天发送不在回放中执行；通过发送判断不等于工作完成或已经送达。<br><a href="model-replay/results.json">完整结果 JSON</a> · <a href="run-notes.md">运行说明</a>。反馈存储键保持不变，不删除你之前留下的意见。</footer>',html,flags=re.S)
    if edits:
        html = html.replace('这里只显示模型实际输出，未人工替换为理想答案。',
            '右侧是本次逐条修订稿，不是自动系统已稳定产出这些回复的证明。自动回放原文、失败和检查记录单独保留，没有被修订稿覆盖。')
    target.write_text(html)
    target.with_name("display-cases.json").write_text(json.dumps(cases, ensure_ascii=False, indent=2) + "\n")
    return {"cases": len(cases), "checks": check_count, "revisions": rejected, "fallbacks": fallback, "protected_drops": protected, "errors": errors}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--template", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--edits", type=Path, help="Explicit review edits, displayed separately from unchanged replay output")
    args = parser.parse_args()
    print(json.dumps(build(args.template, args.report, args.out, args.edits), ensure_ascii=False))
