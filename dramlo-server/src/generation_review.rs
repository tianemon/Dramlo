use super::*;

fn mentioned(prompt: &str, name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() { return false; }
    if !name.is_ascii() && name.chars().count() > 1 { return prompt.contains(name); }
    let token = |ch: char| if name.is_ascii() { ch.is_ascii_alphanumeric() || ch == '_' }
        else { ch.is_alphanumeric() || ch == '_' };
    let prompt = prompt.to_lowercase();
    let name = name.to_lowercase();
    prompt.match_indices(&name).any(|(start, matched)| {
        !prompt[..start].chars().next_back().is_some_and(token)
            && !prompt[start + matched.len()..].chars().next().is_some_and(token)
    })
}

pub(super) fn review_cast(
    project: &Value, shot: &Value, prompt: &str, refs: Option<&[String]>, override_reason: Option<&Value>,
) -> Result<Vec<String>> {
    let bound = shot["subjectIds"].as_array().map(Vec::as_slice).unwrap_or_default();
    let mut issues = vec![];
    for subject in project["subjects"].as_array().into_iter().flatten() {
        let (Some(id), Some(name)) = (subject["id"].as_str(), subject["name"].as_str()) else { continue };
        let bound_here = bound.iter().any(|item| item == id);
        let named_here = mentioned(prompt, name) || subject["promptName"].as_str().is_some_and(|name| mentioned(prompt,name));
        let bound_with_board = bound_here && subject["designSheetAssetId"].is_string();
        let review_board = bound_with_board && refs.is_some_and(|refs|
            refs.is_empty() || shot["subjectBindingMode"] == "explicit");
        if !named_here && !review_board { continue; }
        if !bound_here {
            issues.push(json!({"code":"subject_not_bound","subjectId":id,"name":name}));
        } else if let Some(refs) = refs {
            let selected = subject["designSheetAssetId"].as_str();
            let known_refs = ["designSheetHistoryIds","referenceAssetIds"].into_iter()
                .flat_map(|key| subject[key].as_array().into_iter().flatten())
                .filter_map(Value::as_str);
            if !selected.into_iter().chain(known_refs).any(|asset| refs.iter().any(|item| item == asset)) {
                issues.push(json!({"code":"missing_subject_reference","subjectId":id,"name":name,"expectedAssetId":selected}));
            }
        }
    }
    if issues.is_empty() { return Ok(vec![]); }
    let names: Vec<_> = issues.iter().filter_map(|item| item["name"].as_str()).collect();
    let message = format!("生成正文提到的人物缺少分镜绑定或本次人物参考图：{}。请对照 issues 补齐；明确只是画外提及、替用参考或有意不用参考时才填写 bindingReview。", names.join("、"));
    if let Some(reason) = override_reason {
        if reason.as_str().map(str::trim).is_some_and(|reason| !reason.is_empty() && reason.encode_utf16().count() <= 500) {
            return Ok(vec![message]);
        }
        return Err(invalid("bindingReview 须为 1 至 500 字的核对原因"));
    }
    Err(invalid(&message).with_issues(issues))
}

pub(super) fn is_grid(layout: &Value) -> bool {
    layout["rows"].as_u64().unwrap_or(1).saturating_mul(layout["columns"].as_u64().unwrap_or(1)) > 1
}

fn board_error(asset_id: Option<&str>) -> Failure {
    invalid("首尾帧需要一张单幅。分格图会被模型带进动画，请放在看板槽，或先裁出单格")
        .with_issues(vec![json!({"code":"board_in_frame_slot","assetId":asset_id})])
}

pub(super) fn validate_frame_layout(kind: &str, layout: Option<&Value>) -> Result<()> {
    if matches!(kind,"shot-first"|"shot-last") && layout.is_some_and(is_grid) { return Err(board_error(None)); }
    Ok(())
}

fn is_board(project: &Value, assets: &Value, asset_id: &str) -> bool {
    assets.as_array().into_iter().flatten().any(|asset| asset["id"] == asset_id &&
        (asset["frameRole"] == "board" || asset["imageGeneration"]["targetType"] == "shot-board")) ||
        project["shots"].as_array().into_iter().flatten().any(|shot|
            shot["boardAssetId"] == asset_id || shot["boardHistoryIds"].as_array().into_iter().flatten().any(|id| id == asset_id)
            || (shot["imageAssetId"] == asset_id && is_grid(&shot["imagePromptLayout"])))
}

pub(super) fn validate_frame(app: &App, file: &Path, project: &Value, asset_id: &str) -> Result<()> {
    if is_board(project,&studio_mcp::manifest(app,file)?,asset_id) { return Err(board_error(Some(asset_id))); }
    Ok(())
}

pub(super) fn validate_frame_changes(app: &App, file: &Path, previous: &Value, project: &Value) -> Result<()> {
    let mut added = std::collections::BTreeSet::new();
    for path in ["/shots","/canvas/nodes"] {
        let previous: std::collections::HashMap<_,_> = previous.pointer(path).and_then(Value::as_array)
            .into_iter().flatten().filter_map(|item| item["id"].as_str().map(|id|(id,item))).collect();
        let canvas = path == "/canvas/nodes";
        for item in project.pointer(path).and_then(Value::as_array).into_iter().flatten() {
            let old = item["id"].as_str().and_then(|id| previous.get(id).copied()).unwrap_or(&Value::Null);
            let (before,after) = if canvas { (&old["data"],&item["data"]) } else { (old,item) };
            for key in ["firstFrameAssetId","lastFrameAssetId"] {
                if before[key] != after[key] {
                    if let Some(id) = after[key].as_str() { added.insert(id); }
                }
            }
        }
    }
    if added.is_empty() { return Ok(()); }
    let assets = studio_mcp::manifest(app,file)?;
    for id in added {
        if is_board(previous,&assets,id) || is_board(project,&assets,id) { return Err(board_error(Some(id))); }
    }
    Ok(())
}

pub(super) fn reference_facts(project: &Value, shot: &Value, submitted: &[String]) -> Value {
    let contains = |asset: &str| submitted.iter().any(|id| id == asset);
    let subjects: Vec<_> = shot["subjectIds"].as_array().into_iter().flatten().filter_map(|id| {
        let subject = project["subjects"].as_array()?.iter().find(|item| item["id"] == *id)?;
        let selected = subject["designSheetAssetId"].as_str();
        let submitted = selected.into_iter().chain(["designSheetHistoryIds","referenceAssetIds"].into_iter()
            .flat_map(|key| subject[key].as_array().into_iter().flatten()).filter_map(Value::as_str)).find(|asset| contains(asset));
        Some(json!({"subjectId":id,"name":subject["name"],"promptName":subject["promptName"],
            "selectedAssetId":selected,"submittedAssetId":submitted}))
    }).collect();
    let scene = shot["sceneId"].as_str().and_then(|id| project["scenes"].as_array()?.iter().find(|item| item["id"] == id))
        .map(|scene| {
            let selected = scene["plateAssetId"].as_str();
            json!({"sceneId":scene["id"],"name":scene["name"],"selectedAssetId":selected,
                "submittedAssetId":selected.filter(|id| contains(id))})
        });
    json!({"subjects":subjects,"scene":scene})
}

pub(super) fn workload(target: &str, requests: u64, image_inputs: usize, video_seconds: f64, size: Value) -> Value {
    json!({"target":target,"modelRequests":requests,"imageInputs":image_inputs,"videoSeconds":video_seconds,
        "requestedSize":size,"money":null,"moneyStatus":"not_estimated"})
}

pub(super) fn sum_workload<'a>(items: impl Iterator<Item=&'a Value>) -> Value {
    let (mut requests,mut images,mut seconds,mut steps,mut unknown) = (0_u64,0_u64,0.0_f64,0_u64,0_u64);
    for item in items {
        if !item.is_object() { unknown += 1; continue; }
        requests += item["modelRequests"].as_u64().unwrap_or_default();
        images += item["imageInputs"].as_u64().unwrap_or_default();
        seconds += item["videoSeconds"].as_f64().unwrap_or_default();
        steps += 1;
    }
    json!({"estimatedSteps":steps,"unestimatedSteps":unknown,"modelRequests":requests,"imageInputs":images,"videoSeconds":seconds,
        "money":null,"moneyStatus":"not_estimated"})
}
