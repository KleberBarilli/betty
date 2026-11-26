(function(root) {
  const clean = value => String(value || '').replace(/\s+/g, ' ').trim();
  function ordinal(value) {
    const n=Number(value), last=n%100;
    return `${n}${last>=11&&last<=13?'th':({1:'st',2:'nd',3:'rd'}[n%10]||'th')}`;
  }
  function period(text) {
    const s = clean(text);
    if (/\b(?:first|1st)\s*(?:5|five)\s+innings?\b/i.test(s)) return '1st 5 Innings';
    const range=s.match(/\b(?:first|1st)\s+(\d+)\s+innings\b/i);
    if(range) return `1st ${Number(range[1])} Innings`;
    const inning = s.match(/\b(\d+)(?:st|nd|rd|th|º|°)\s+inning\b/i);
    if (inning) return `${ordinal(inning[1])} Inning`;
    const namedInning=s.match(/\b(first|second|third|fourth|fifth|sixth|seventh|eighth|ninth)\s+inning\b/i);
    if(namedInning) return `${ordinal(['first','second','third','fourth','fifth','sixth','seventh','eighth','ninth'].indexOf(namedInning[1].toLowerCase())+1)} Inning`;
    const quarter = s.match(/\b([1-4])Q\b|\b([1-4])(?:st|nd|rd|th)\s+quarter\b|(?:^|_)q([1-4])(?:_|$)|\b([1-4])[º°]\s+quarto\b/i);
    if (quarter) {
      const number = quarter[1] || quarter[2] || quarter[3] || quarter[4];
      return Number(number) === 1 ? '1st Quarter' : `${number}º quarto`;
    }
    if (/\b(?:first|1st)\s+half\b|\b1H\b|(?:first|1st)_half/i.test(s)) return '1st Half';
    if (/\b(?:second|2nd)\s+half\b|\b2H\b|(?:second|2nd)_half/i.test(s)) return '2nd Half';
    return '';
  }
  function unit(text, slug = '') {
    const s = clean(text);
    if (/\b(?:corners?|corner kicks?|escanteios)\b/i.test(s)) return 'escanteios';
    if (/\b(?:rebounds?|rebotes)\b/i.test(s)) return 'rebotes';
    if (/\brounds?\b/i.test(s)) return 'rounds';
    if (/\b(?:goals?|gols)\b/i.test(s)) return 'gols';
    if (/\b(?:runs?|baseball|mlb)\b/i.test(s)) return 'runs';
    if (/\b(?:points?|pontos|basketball|basquete|american football|nfl)\b/i.test(s)) return 'pontos';
    if (/^(?:nba|wnba|ncaab|cbb|bkbsl|fiba|euroleague|cfb|ncaaf|nfl|cfl|ufl|xfl)-/i.test(slug)) return 'pontos';
    if (/^(?:mlb|npb|kbo|cpbl|milb)-/i.test(slug)) return 'runs';
    return '';
  }
  function sportFromKalshiRules(market) {
    const rules=clean(market?.rules_primary);
    if(!rules)return null;
    // Only an explicit modality attached to game/match is evidence. Team
    // names, ticker fragments and mentions of another league are not proof.
    const patterns=[
      ['American Football',/\b(?:american|college|pro|professional)\s+football\s+game\b/i],
      ['Basketball',/\bbasketball\s+(?:game|match)\b/i],
      ['Baseball',/\bbaseball\s+(?:game|match)\b/i],
      ['Hockey',/\b(?:ice\s+)?hockey\s+(?:game|match)\b/i],
      ['Tennis',/\btennis\s+(?:game|match)\b/i],
      ['Soccer',/\bsoccer\s+(?:game|match)\b/i],
      ['Rugby',/\brugby\s+(?:game|match)\b/i],
      ['Cricket',/\bcricket\s+(?:game|match)\b/i],
      ['Volleyball',/\bvolleyball\s+(?:game|match)\b/i]
    ];
    const found=patterns.filter(([,pattern])=>pattern.test(rules));
    return found.length===1?found[0][0]:null;
  }
  function sportUnit(sport) {
    const name = clean(sport).toLowerCase();
    if (['basquete', 'futebol americano', 'basketball', 'american football'].includes(name)) return 'pontos';
    if (['beisebol', 'baseball'].includes(name)) return 'runs';
    if (['futebol', 'hóquei', 'soccer', 'hockey'].includes(name)) return 'gols';
    return '';
  }
  function footballWinner(team, scope = '', side = 'yes') {
    let p = period(scope);
    p = p.replace(/^([1-4])º quarto$/, (_, n) => `${n}${['','st','nd','rd','th'][Number(n)]} Quarter`);
    return [p, clean(team), 'ML', side === 'no' ? 'No' : ''].filter(Boolean).join(' ');
  }
  function inningWinnerTitle(row) {
    if (String(row?._platform || '').toLowerCase() !== 'kalshi') return '';
    const ticker=String(row.slug || row.ticker || row.market_ticker || '').toUpperCase();
    const match=ticker.match(/^KX(?:MLB|NPB|KBO|CPBL|MILB|WBC)INNINGWIN-[^-]+-([1-9]\d*)-([A-Z0-9]+)$/);
    const side=String(row.outcome || '').toLowerCase();
    if (!match || !['yes','no'].includes(side)) return '';
    const raw=clean(row.title), split=raw.indexOf(' - ');
    if(split<0) return '';
    const matchup=raw.slice(0,split), teams=matchup.split(/\s+vs\.?\s+/i);
    if(teams.length!==2) return '';
    const detail=raw.slice(split+3);
    const meta=row._kalshiTitleMeta?.market;
    const candidates=[meta?.title?.replace(/\s+wins?\??$/i,''),meta?.yes_sub_title,
      detail.replace(/^\d+(?:st|nd|rd|th) Inning\s+/i,'').replace(/\s+ML(?:\s+No)?$/i,'').replace(/\s+No$/i,'')];
    const norm=s=>clean(s).toLowerCase().replace(/[^a-z0-9]/g,'');
    const selected=teams.filter(team=>candidates.some(c=>norm(c)===norm(team)));
    // Never guess a team from an abbreviation or invert a No into its opponent.
    if(selected.length!==1) return '';
    return `${matchup} - ${ordinal(match[1])} Inning ${selected[0]} ML${side==='no'?' No':''}`;
  }
  function drawScope(value) {
    const words=['first','second','third','fourth','fifth','sixth','seventh','eighth','ninth'];
    const text=clean(value).replace(/\b(?:including|excluding)\s+(?:overtime|extra time)\b/ig,'')
      .replace(/\b(first|second|third|fourth|fifth|sixth|seventh|eighth|ninth)\s+(quarter|period|set|map)\b/ig,(_,n,unit)=>`${ordinal(words.indexOf(n.toLowerCase())+1)} ${unit}`);
    const p=period(text).replace(/^([1-4])º quarto$/,(_,n)=>`${ordinal(n)} Quarter`);
    if(p)return p;
    const numbered=text.match(/\b(\d+)(?:st|nd|rd|th)\s+(Period|Set|Map)\b|\b(Set|Map)\s+(\d+)\b/i);
    if(numbered){const n=Number(numbered[1]||numbered[4]),unit=numbered[2]||numbered[3];if(n>0)return `${ordinal(n)} ${unit[0].toUpperCase()+unit.slice(1).toLowerCase()}`;}
    if(/\bextra innings\b/i.test(text))return 'Extra Innings';
    if(/^(?:overtime|extra time)(?: winner)?$/i.test(text))return /^extra/i.test(text)?'Extra Time':'Overtime';
    if(/^(?:the\s+)?(?:game|match|full game|regulation(?: time)?)(?: winner)?\??$/i.test(text))return '';
    return null;
  }
  function drawDetail(question, outcome, scope = '') {
    const q=clean(question),tail=clean(q.split(':').at(-1)),side=clean(outcome);
    if(!/^(Yes|No|Tie|Draw)$/i.test(side))return '';
    const neither=tail.match(/^(?:Will\s+)?neither team wins?(?:\s+(?:the\s+)?(.+?))?\??$/i);
    const tied=tail.match(/^(?:Will\s+)?(?:the\s+)?(?:teams|both teams|game|match)\s+(?:tie(?:s)?(?:\s+scoring)?|draw|(?:end|finish)\s+in\s+(?:a\s+)?(?:tie|draw))(?:\s+(?:in|at|after)\s+(?:the\s+)?(.+?))?\??$/i);
    const simple=/^(?:Tie|Draw)\??$/i.test(tail);
    const selectedDraw=/^(?:Tie|Draw)$/i.test(side) && /\b(?:winner|result)\b/i.test(q);
    if(!neither && !tied && !simple && !selectedDraw)return '';
    // "after the 5th inning" can be cumulative, unlike scoring IN an inning.
    if(/\bafter\s+(?:the\s+)?\d+(?:st|nd|rd|th)\s+inning\b/i.test(q))return '';
    const explicit=clean(neither?.[1]||tied?.[1]);
    let p=drawScope(explicit || q);
    if(p===null && !explicit)p=drawScope(scope);
    if(p===null)return '';
    return `${p?p+' ':''}Empate${/^No$/i.test(side)?' No':''}`;
  }
  function selectionDetail(question, outcome, scope = '') {
    const q=clean(question),tail=clean(q.split(':').at(-1));
    const selected=clean(outcome),yn=/^(yes|no)$/i.test(selected);
    const negative=/^no$/i.test(selected)?' No':'';
    const p=period(q+' '+scope).replace(/^([1-4])º quarto$/,(_,n)=>`${ordinal(n)} Quarter`);
    const prefix=p?p+' ':'';
    const draw=drawDetail(q,selected,scope);
    if(draw)return draw;
    const validTeam=team=>team && !/^(?:Yes|No|Over|Under|Draw|Tie|Neither(?: team)?|None|No team)$/i.test(team) && !/\s+vs\.?\s+/i.test(team);
    const neitherRace=tail.match(/^(?:Will\s+)?neither team\s+(?:reach|score)\s+(\d+)\s+points?(?:\s+first)?\??$/i);
    if(neitherRace && yn && Number(neitherRace[1])>0)return `${prefix}Qualquer equipe Race to ${Number(neitherRace[1])} pontos${/^Yes$/i.test(selected)?' No':''}`;
    const race=tail.match(/^(?:Will\s+)?(.+?)\s+(?:reach|score)\s+(\d+)\s+points?\s+first\??$/i);
    const genericRace=tail.match(/^(?:Race to|First to)\s+(\d+)\s+(?:points?|pontos)\??$/i);
    if(race || genericRace){
      const team=yn?clean(race?.[1]):selected,n=Number(race?.[2]||genericRace?.[1]);
      if(genericRace && !yn && /^(?:Neither(?: team)?|None|No team)$/i.test(selected) && n>0)return `${prefix}Qualquer equipe Race to ${n} pontos No`;
      if(validTeam(team) && n>0 && (!race || yn || team.toLowerCase()===clean(race[1]).toLowerCase()))return `${prefix}${team} Race to ${n} pontos${negative}`;
    }
    const firstTD=tail.match(/^(?:Will\s+)?(.+?)\s+scores?\s+(?:the\s+)?(?:first|1st)\s+(?:TD|touchdown)(?:\s+in\s+(?:the\s+)?(.+?))?\??$/i);
    const genericTD=/^(?:First Team to Score (?:a )?(?:TD|Touchdown)|First Touchdown Scorer)\??$/i.test(tail);
    if(firstTD || genericTD){
      const team=yn?clean(firstTD?.[1]):selected;
      const explicitPeriod=firstTD?.[2];
      if(validTeam(team) && (!explicitPeriod || period(explicitPeriod)) && (!firstTD || yn || team.toLowerCase()===clean(firstTD[1]).toLowerCase()))return `${prefix}${team} First TD${negative}`;
    }
    const firstInnings=tail.match(/^(?:Will\s+)?(.+?)\s+(?:wins?\s+(?:the\s+)?)?(?:first|1st)\s+(\d+|five)\s+innings?(?:\s+winner)?\??$/i);
    if(firstInnings && yn){
      const team=clean(firstInnings[1]),n=/five/i.test(firstInnings[2])?5:Number(firstInnings[2]);
      if(validTeam(team)&&n>0)return `1st ${n} Innings ${team} ML${negative}`;
    }
    const scopedInnings=q.match(/^(.+?)\s+wins?\??$/i);
    if(yn && /^1st \d+ Innings$/.test(p) && scopedInnings && validTeam(scopedInnings[1]))return `${p} ${clean(scopedInnings[1])} ML${negative}`;
    return '';
  }
  function compactKalshiTotal(title, side, scope = '') {
    const text=clean(title),yn=clean(side).toLowerCase();
    if(!['yes','no'].includes(yn))return '';
    const scopePattern='(?:Full Game|Reg(?:ulation)? Time|(?:first|1st|second|2nd) Half|[1-4](?:st|nd|rd|th) Quarter|[1-4]Q|[12]H|(?:first|1st) (?:\\d+|five) Innings?|\\d+(?:st|nd|rd|th) (?:Inning|Period|Set|Map)|Overtime|Extra Time)';
    const colon=text.lastIndexOf(':');
    const prefix=colon<0?'':text.slice(0,colon);
    if(prefix && !new RegExp('^'+scopePattern+'$','i').test(prefix))return '';
    const question=(colon<0?text:text.slice(colon+1).trim()).replace(/^Will\s+(?:there be\s+)?/i,'');
    const units='points?|goals?|runs?|corners?|corner kicks?|games?|sets?|rounds?|rebounds?|assists?|strikeouts?|steals?|blocks?|shots on target|three.pointers?';
    const total=question.match(new RegExp('^(?:(.+?)\\s+(?:scores?\\s+)?)?(over|under)\\s+(\\d+(?:[.,]\\d+)?)\\s+(?:('+scopePattern+')\\s+)?(?:total\\s+)?('+units+')(?:\\s+(?:be\\s+)?(?:scored|recorded))?(?:\\s+in\\s+(?:the\\s+)?('+scopePattern+'))?\\??$','i'));
    if(!total)return '';
    const team=clean(total[1]);
    if(team && /\b(?:if|and|or|not|no|neither|either|both|any|each|all|at least|first|race|win|reach|than|exactly|margin|consecutive|combined|total)\b|\bvs\.?\b|[?:]/i.test(team))return '';
    const scopeName=s=>period(s) || drawScope(s) || '';
    const explicitScopes=[prefix,total[4],total[6]].filter(Boolean).map(scopeName);
    if(new Set(explicitScopes).size>1)return '';
    const p=explicitScopes.length?explicitScopes[0]:scopeName(scope);
    const line=total[3].replace(',','.'),countUnit=total[5].toLowerCase();
    // Points/goals/runs are discrete. At a fractional line the opposite side
    // is an exact complement; at an integer line the equality case differs.
    const inverse=yn==='no' && !Number.isInteger(Number(line)) && !/^rounds?$/.test(countUnit);
    const ou=inverse ? (/^over$/i.test(total[2])?'Under':'Over') : total[2][0].toUpperCase()+total[2].slice(1).toLowerCase();
    const label=unit(countUnit)||(/^games?$/.test(countUnit)?'games':/^sets?$/.test(countUnit)?'sets':/^assists?$/.test(countUnit)?'assistências':/^strikeouts?$/.test(countUnit)?'strikeouts':/^steals?$/.test(countUnit)?'roubos':/^blocks?$/.test(countUnit)?'bloqueios':countUnit==='shots on target'?'chutes no alvo':/^three.pointers?$/.test(countUnit)?'cestas de 3':'');
    if(!label)return '';
    return [team,p,ou,line,label,yn==='no'&&!inverse?'No':''].filter(Boolean).join(' ');
  }
  function kalshiDetail(m, side, scope = '', americanFootball = false) {
    if (!m) return '';
    const title = clean(m.title || m.subtitle);
    const question = title.includes(':') ? title.slice(title.lastIndexOf(':') + 1).trim() : title;
    const subtitle = clean(side === 'no' ? m.no_sub_title : m.yes_sub_title);
    const p = period([title, scope, subtitle].join(' '));
    const yn = side === 'yes' ? 'Yes' : side === 'no' ? 'No' : '';
    const special=selectionDetail(title,side,scope);
    if(special)return special;
    const compactTotal=compactKalshiTotal(title,side,scope);
    if(compactTotal)return compactTotal;
    const advancement=question.match(/^(.+?)\s+(?:to advance|advances?)\??$/i);
    if(advancement && yn) return `${clean(advancement[1])} Advances${side==='no'?' No':''}`;
    const matchupWinner=question.match(/^(.+?)\s+vs\.?\s+(.+?)\s+Winner\??$/i);
    if(matchupWinner && yn && /\b(?:soccer|football|basketball|baseball|hockey|tennis|game|match)\b/i.test(String(m.rules_primary||scope))) {
      const selected=clean(m.yes_sub_title);
      const norm=s=>clean(s).normalize('NFD').replace(/[\u0300-\u036f]/g,'').toLowerCase().replace(/[^a-z0-9]/g,'');
      const candidates=[matchupWinner[1],matchupWinner[2]].filter(team=>norm(selected) && norm(team)===norm(selected));
      if(candidates.length===1)return footballWinner(selected,p,side);
    }
    if (americanFootball && yn) {
      const win = question.match(/^(?:Will\s+)?(.+?)\s+win(?:s)?(?:\s+the)?(?:\s+([1-4](?:st|nd|rd|th)\s+quarter|(?:first|second|1st|2nd)\s+half))?\??$/i);
      if (win) return footballWinner(win[1], win[2] || scope, side);
    }
    // RFI contracts are binary run/no-run questions, not arbitrary totals.
    // Require their explicit question rather than guessing from a strike.
    if (yn && /^(?:First Inning Run\??|(?:Will\s+)?(?:either|any)\s+team\s+score\s+(?:a|at least one)\s+run\s+in\s+the\s+(?:first|1st)\s+inning\??)$/i.test(question)) {
      return `1st Inning ${side === 'yes' ? 'Over' : 'Under'} 0.5 runs`;
    }
    if (yn && /^(?:No (?:run|runs) in (?:the )?(?:first|1st) inning\??|(?:Will\s+)?neither team score (?:a )?run in the (?:first|1st) inning\??)$/i.test(question)) {
      return `1st Inning ${side === 'yes' ? 'Under' : 'Over'} 0.5 runs`;
    }
    if (yn && /^(?:Will (?:the )?game go to |)(?:extra innings|Extra Innings)\??$/i.test(question)) return `Extra Innings ${yn}`;
    const inningWinner=question.match(/^(?:Will\s+)?(.+?)\s+win(?:s)?(?:\s+the)?\s+(\d+(?:st|nd|rd|th)\s+inning|(?:first|second|third|fourth|fifth|sixth|seventh|eighth|ninth)\s+inning)\??$/i);
    if(inningWinner && yn) return `${period(inningWinner[2])} ${clean(inningWinner[1])} ML${side === 'no' ? ' No' : ''}`;
    const winner = question.match(/^(?:Will\s+)?(.+?)\s+win(?:s)?(?:\s+the)?\s+(?:[1-4](?:st|nd|rd|th)\s+quarter|(?:first|second|1st|2nd)\s+half)\??$/i);
    if (winner && p) {
      if (p === '1st Half' && yn) return `${p} ${clean(winner[1])} ML${side === 'no' ? ' No' : ''}`;
      return `${clean(winner[1])} ${p}${yn ? ' ' + yn : ''}`;
    }
    const sources = [question, subtitle];
    let total = null;
    for (const text of sources) {
      const match = text.match(/\b(over|under)\s+(\d+(?:[.,]\d+)?)\s+(?:(?:[1-4]Q|[12]H)\s+)?(goals?|runs?|points?)\b/i);
      if (match) { total = match; break; }
    }
    if (!total) return '';
    // A binary No is not a second Over; keep the contract explicit instead of
    // assuming how pushes/cancellations map to a complementary total.
    if (side === 'no' && !(p && /Inn/i.test(p) && /^runs?$/i.test(total[3]))) return `${p && !period(question) ? p + ' ' : ''}${question.replace(/\?$/, '')}: No`;
    const before = clean(total.input.slice(0, total.index)).replace(/^(?:Will(?:\s+|$)|Reg(?:ulation)?\s*Time\s*:\s*)/i, '').replace(/\s+(?:score|scores)\s*$/i, '');
    const periodOnly = before.replace(/:\s*$/,'').trim();
    const team = before && !(p && period(periodOnly)===p && /^(?:first|1st|second|2nd|third|3rd|\d+)/i.test(periodOnly)) && !/^(?:Full Game|First 5 innings|1st Half|2nd Half)\s*:?$/i.test(before) && !/^(?:will there be|there be)$/i.test(before) ? before.replace(/:\s*$/, '') : '';
    const ou = total[1][0].toUpperCase() + total[1].slice(1).toLowerCase();
    return [team, p, ou, total[2].replace(',', '.'), unit(total[3]), side === 'no' ? 'No' : ''].filter(Boolean).join(' ');
  }
  function drawTitle(d) {
    if(String(d?._platform||'polymarket').toLowerCase()!=='polymarket')return '';
    const own=exactMarket(d),question=clean(own?.question || d.title),side=clean(d.outcome);
    if(!/^(Yes|No)$/i.test(side))return '';
    const match=question.match(/^(?:Will\s+)?(.+?)\s+vs\.?\s+(.+?)\s+(?:end|finish)\s+in\s+a\s+draw(?:\s+(?:in|at)\s+(?:the\s+)?(.+?))?\??$/i);
    if(!match)return '';
    const type=clean(own?.sportsMarketType).toLowerCase();
    const scope=drawScope(match[3]||'') || (/first_half|halftime/.test(type)?'1st Half':/second_half/.test(type)?'2nd Half':'');
    if(match[3] && !scope)return '';
    return `${clean(match[1])} vs ${clean(match[2])} - Empate${scope?' '+scope:''}${/^No$/i.test(side)?' No':''}`;
  }
  function extendedTitle(d, matchupHint = '') {
    const m=exactMarket(d);
    if (!m) return '';
    const type=clean(m.sportsMarketType).toLowerCase().replace(/[\s-]+/g,'_');
    const q=clean(m.question), outcome=clean(d.outcome);
    if (!type || !q || !outcome) return '';
    const yn=/^(yes|no)$/i.test(outcome)?outcome[0].toUpperCase()+outcome.slice(1).toLowerCase():'';
    const matchupSource=[d._titleDisplay,d.title,q].find(v=>/\s+vs\.?\s+/i.test(clean(v).split(' - ')[0]));
    let base=clean(matchupHint || matchupSource || d._titleDisplay || d.title).split(' - ')[0].replace(/\s+vs\.\s+/i,' vs ');
    const teams=base.match(/^(.+?)\s+vs\s+(.+)$/i);
    if(teams)base=`${teams[1].split(':').at(-1).trim()} vs ${teams[2].split(':')[0].trim()}`;
    const prefix=base ? base+' - ' : '';
    const fallback=()=>`${prefix}Mercado não confirmado: ${q} — Seleção: ${outcome}`;
    if(/^(?:moneyline|(?:first_half|second_half|q[1-4]|(?:first|second|third|fourth)_quarter)_moneyline|soccer_halftime_result|rugby_halftime_result|rugby_second_half_result|baseball_team_first_five_winner|hockey_team_period[1-3]_winner)$/.test(type)){
      let drawTypeScope='Game';
      if(/first_half|halftime/.test(type))drawTypeScope='1st Half';
      else if(/second_half/.test(type))drawTypeScope='2nd Half';
      else if(/first_five/.test(type))drawTypeScope='1st 5 Innings';
      else {
        const quarter=type.match(/q([1-4])|^(first|second|third|fourth)_quarter/);
        const hockey=type.match(/period([1-3])/);
        if(quarter)drawTypeScope=`${ordinal(quarter[1]||['first','second','third','fourth'].indexOf(quarter[2])+1)} Quarter`;
        else if(hockey)drawTypeScope=`${ordinal(hockey[1])} Period`;
      }
      const draw=drawDetail(q,outcome,drawTypeScope);
      if(draw)return prefix+draw;
    }
    if(/_race_to_points$|_first_td(?:_team)?$|_first_touchdown(?:_scorer)?$/.test(type)){
      const detail=selectionDetail(q,outcome);
      const raceLine=detail.match(/\bRace to (\d+) pontos/);
      if(raceLine && m.line!=null && Number(m.line)!==Number(raceLine[1]))return fallback();
      return detail?prefix+detail:fallback();
    }
    if(/^tennis_(?:(?:first|second|third|fourth|fifth)_)?set_games_totals$/.test(type)){
      const number=q.match(/\bSet\s+(\d+)\b|\b(\d+)(?:st|nd|rd|th)\s+Set\b/i);
      const named=type.match(/^tennis_(first|second|third|fourth|fifth)_set/);
      const n=Number(number?.[1]||number?.[2]||(named?['first','second','third','fourth','fifth'].indexOf(named[1])+1:0));
      const line=q.match(/\b(O\/U|Over\/Under|Over|Under)\s+(\d+(?:[.,]\d+)?)/i);
      const ou=/^(Over|Under)$/i.test(outcome)?outcome:yn && line && /^(Over|Under)$/i.test(line[1])?line[1]:'';
      if(!n||!line||!ou || (m.line!=null && Number(m.line)!==Number(line[2].replace(',','.'))))return fallback();
      return `${prefix}${ordinal(n)} Set ${ou[0].toUpperCase()+ou.slice(1).toLowerCase()} ${line[2].replace(',','.')} games${yn==='No'?' No':''}`;
    }
    const win=(scope)=>{
      if(!scope)return fallback();
      const named=q.match(/^(?:Will\s+)?(.+?)\s+win(?:s)?\b/i);
      const team=yn?clean(named?.[1]):outcome;
      return team && !/^(Over|Under|Draw)$/i.test(team)?`${prefix}${scope} ${team} ML${yn==='No'?' No':''}`:fallback();
    };
    const inning=type.match(/^baseball_team_inning([1-9]\d*)_winner$/);
    if(inning)return win(`${ordinal(inning[1])} Inning`);
    const hockey=type.match(/^hockey_team_period([1-3])_winner$/);
    if(hockey)return win(`${ordinal(hockey[1])} Period`);
    if(['tennis_set_winner','tennis_first_set_winner','volleyball_set_winner','child_moneyline'].includes(type)){
      const map=type==='child_moneyline';
      const n=type==='tennis_first_set_winner'?'1':q.match(map?/\bMap\s+(\d+)\b|\b(\d+)(?:st|nd|rd|th)\s+Map\b/i:/\bSet\s+(\d+)\b|\b(\d+)(?:st|nd|rd|th)\s+Set\b/i);
      const number=typeof n==='string'?n:n?.[1]||n?.[2];
      return win(number?`${ordinal(number)} ${map?'Map':'Set'}`:'');
    }
    if(/^both_teams_to_score(?:_(?:first|second)_half)?$/.test(type)){
      const p=/second_half$/.test(type)?'2nd Half':/first_half$/.test(type)?'1st Half':period(q);
      return yn?`${prefix}Both Teams to Score${p?' '+p:''} ${yn}`:fallback();
    }
    if(['rugby_halftime_result','rugby_second_half_result'].includes(type))return win(type==='rugby_halftime_result'?'1st Half':'2nd Half');
    const props={assists:'Assistências',soccer_player_shots_on_target:'Chutes no alvo',baseball_player_strikeouts:'Strikeouts'};
    if(props[type]){
      const player=clean(q.split(':')[0]);
      const line=q.match(/\b(?:O\/U|Over\/Under|Over|Under)\s+(\d+(?:[.,]\d+)?)/i);
      const ou=outcome.match(/^(Over|Under)$/i);
      if(!q.includes(':')||!player||!line||(!yn&&!ou))return fallback();
      const selection=yn?`O/U ${line[1].replace(',','.')} ${yn}`:`${ou[1][0].toUpperCase()+ou[1].slice(1).toLowerCase()} ${line[1].replace(',','.')}`;
      // Yes/No on a directional question is not Yes/No on an O/U proposition.
      if(yn&&!/\bO\/U|Over\/Under/i.test(q))return fallback();
      return `${prefix}${player} ${props[type]} ${selection}`;
    }
    if(['cricket_first_inning_runs','cricket_second_inning_runs','first_innings_totals','second_innings_totals'].includes(type)){
      const scope=/second/.test(type)?'2nd Innings':'1st Innings';
      const line=q.match(/\b(?:O\/U|Over\/Under|Over|Under)\s+(\d+(?:[.,]\d+)?)/i);
      const ou=outcome.match(/^(Over|Under)$/i);
      if(!line||!ou)return fallback();
      return `${prefix}${scope} ${ou[1][0].toUpperCase()+ou[1].slice(1).toLowerCase()} ${line[1].replace(',','.')} runs`;
    }
    if(['ufc_method_of_victory','ufc_method_of_finish','combat_method_of_victory'].includes(type)){
      const method=q.match(/^(?:Will\s+)?(.+?)\s+win\s+by\s+(knockout|KO\/TKO|TKO|KO|submission|decision)\??$/i);
      const names={knockout:'nocaute',ko:'KO',tko:'TKO','ko/tko':'KO/TKO',submission:'finalização',decision:'decisão'};
      return method&&yn?`${prefix}${clean(method[1])} Vitória por ${names[method[2].toLowerCase()]} ${yn}`:fallback();
    }
    if(type==='f1_race_podium'){
      const driver=q.match(/^(?:Will\s+)?(.+?)\s+finish\s+on\s+the\s+podium\??$/i);
      return driver&&yn?`${prefix}${clean(driver[1])} Pódio ${yn}`:fallback();
    }
    if(type==='moneyline' && /\bwin\s+the\s+tournament\??$/i.test(q)){
      const golfer=q.match(/^(?:Will\s+)?(.+?)\s+win\s+the\s+tournament\??$/i);
      return golfer&&yn?`${prefix}Torneio ${clean(golfer[1])} ML${yn==='No'?' No':''}`:fallback();
    }
    // Unknown structured types must never reach the generic sport-unit/ML guesses.
    const legacy=/^(?:moneyline|spreads?|totals?|team_totals(?:_home|_away)?|team_totals_(?:home|away)|game_team_totals|h2_team_totals|total_corners|total_goals|both_teams_to_score|rebounds|map_handicap|tennis_(?:set_handicap|game_handicap|set_totals|match_totals|first_set_totals)|baseball_(?:team_first_five_(?:spread|total|winner)|game_extra_innings)|nrfi|soccer_(?:team_to_advance|halftime_result|team_totals|home_team_totals|away_team_totals|team_total_corners|first_half_team_totals|second_half_team_totals|first_half_total_corners|second_half_total_corners)|(?:first_half|second_half|q[1-4])_(?:moneyline|spreads|totals|team_totals)|football_team_(?:first|second)_half_winner|football_team_(?:first|second|third|fourth)_quarter_winner|p[1-3]_totals)$/;
    return legacy.test(type) || /^(?:to_advance|(?:first|second|third|fourth)_quarter_(?:moneyline|totals|spreads))$/.test(type)?'':fallback();
  }
  function kalshiSpecialTitle(d) {
    const meta=d?._kalshiTitleMeta;
    if(String(d?._platform||'').toLowerCase()!=='kalshi'||!meta?.market)return '';
    const q=clean(meta.market.title),scope=clean(meta.scope),source=q+' '+scope;
    const detail=compactKalshiTotal(q,d.outcome,scope)||selectionDetail(q,d.outcome,scope);
    if(detail){
      const raw=clean(d.title).split(' - ')[0];
      const rules=clean(meta.market.rules_primary);
      const official=rules.match(/\b(?:in|of)\s+(?:the\s+)?(.+?\s+vs\.?\s+.+?)\s+(?:Pro Football|college football|professional baseball|Pro Basketball|college basketball)\s+game\b/i)?.[1];
      const confirmed=clean(d._titleDisplay).split(' - ')[0];
      const matchup=[confirmed,clean(meta.matchup),clean(official),raw].find(t=>/\s+vs\.?\s+/i.test(t)&&!/[?:]/.test(t));
      if(matchup)return `${matchup.replace(/\s+vs\.\s+/i,' vs ')} - ${detail}`;
    }
    let type='';
    const periodNumber=source.match(/\b([1-3])(?:st|nd|rd)\s+period\b/i);
    if(periodNumber && /\bwins?\b/i.test(q))type=`hockey_team_period${periodNumber[1]}_winner`;
    else if(/\bwins?\b.*\b(?:Set\s+\d+|\d+(?:st|nd|rd|th)\s+Set)\b/i.test(q))type='tennis_set_winner';
    else if(/\bwins?\b.*\b(?:Map\s+\d+|\d+(?:st|nd|rd|th)\s+Map)\b/i.test(q))type='child_moneyline';
    else if(/\bwin by (?:knockout|KO\/TKO|TKO|KO|submission|decision)\??$/i.test(q))type='combat_method_of_victory';
    else if(/\bfinish on the podium\??$/i.test(q))type='f1_race_podium';
    else if(q.includes(':') && /\b(?:O\/U|Over\/Under|Over|Under)\s+\d/i.test(q)){
      if(/\bshots on target\b/i.test(q))type='soccer_player_shots_on_target';
      else if(/\bstrikeouts\b/i.test(q))type='baseball_player_strikeouts';
      else if(/\bassists\b/i.test(q))type='assists';
    }
    if(!type)return '';
    const matchup=clean(d.title).split(' - ')[0];
    const synthetic={...d,_sportMeta:{markets:[{slug:d.slug||'exact',question:q,sportsMarketType:type}]},slug:d.slug||'exact'};
    return extendedTitle(synthetic,matchup);
  }
  function exactMarket(d, meta = d?._sportMeta) {
    const markets = Array.isArray(meta?.markets) ? meta.markets : [];
    const asset = clean(d?.asset || d?.assetId || d?.tokenId).replace(/^"|"$/g, '');
    const condition = clean(d?.conditionId || d?.condition_id).toLowerCase();
    const slug = clean(d?.slug).toLowerCase();
    const tests = [];
    if (asset) tests.push(m => (m.tokenIds || []).some(t => clean(t).replace(/^"|"$/g, '') === asset));
    if (condition) tests.push(m => clean(m.conditionId || m.condition_id).toLowerCase() === condition);
    if (slug) tests.push(m => clean(m.slug).toLowerCase() === slug);
    for (const test of tests) { const found = markets.filter(test); if (found.length === 1) return found[0]; }
    return null;
  }
  function combatTitle(d) {
    const m=exactMarket(d);
    if(!m)return '';
    const q=clean(m.question);
    const total=q.match(/\b(Over|Under|O\/U)\s+(\d+(?:[.,]\d+)?)\s+rounds?\b/i);
    if(!total)return '';
    const outcome=clean(d.outcome);
    let side=/^(Over|Under)$/i.test(outcome)?outcome[0].toUpperCase()+outcome.slice(1).toLowerCase():'';
    let negative='';
    if(!side && /^(Yes|No)$/i.test(outcome) && /^(Over|Under)$/i.test(total[1])) {
      side=total[1][0].toUpperCase()+total[1].slice(1).toLowerCase();
      negative=/^No$/i.test(outcome)?' No':'';
    }
    if(!side)return '';
    const candidates=[d.title,...(d._sportMeta?.texts || [])];
    const base=candidates.map(clean).find(t=>/\s+vs\.?\s+/i.test(t));
    if(!base)return '';
    const matchup=base.replace(/^(?:UFC|Bellator|PFL|Boxing|Boxe)[^:]*:\s*/i,'')
      .replace(/\s+-\s+(?:Over|Under|O\/U)\b.*$/i,'').replace(/:\s*(?:Over|Under|O\/U)\b.*$/i,'')
      .replace(/\s+vs\.?\s+/i,' vs ');
    return `${matchup} - ${side} ${total[2].replace(',','.')} rounds${negative}`;
  }
  function kalshiMatchupParts(title) {
    const text=clean(title);
    const prefix=/^(?:the\s+)?(?:first|second|third|fourth|fifth|last|final|\d+(?:st|nd|rd|th))\s+(?:\d+\s+)?(?:half|halves|quarters?|periods?|innings?|sets?|rounds?)\b\s*((?:\([^()]+\)\s*)*)(?:of|in|for)\s+(?:the\s+)?/i;
    const match=text.match(prefix);
    const candidate=match ? text.slice(match[0].length) : text;
    if(!match || !/\S\s+(?:vs\.?|versus|@)\s+\S/i.test(candidate.split(' - ')[0]))return {game:text,qualifier:''};
    return {game:candidate,qualifier:match[1].trim()};
  }
  function cleanKalshiMatchup(title) {
    return kalshiMatchupParts(title).game;
  }
  function kalshiBetQualifier(bet, ...sources) {
    for(const source of sources){
      const {qualifier}=kalshiMatchupParts(source);
      if(!qualifier || bet.toLowerCase().includes(qualifier.toLowerCase()))continue;
      bet = /\s+No$/i.test(bet)
        ? bet.replace(/\s+(No)$/i, ` ${qualifier} $1`)
        : bet + ' ' + qualifier;
    }
    return bet;
  }
  function positionColumns(title, outcome = '', row = null) {
    const text=clean(title);
    const index=text.indexOf(' - ');
    if(index>0 && text.slice(index+3).trim()){
      const game=text.slice(0,index);
      const isKalshi=String(row?._platform||'').toLowerCase()==='kalshi';
      const bet=text.slice(index+3).trim();
      return isKalshi ? {game:cleanKalshiMatchup(game),bet:kalshiBetQualifier(bet,game)} : {game,bet};
    }
    if(row && kalshiTitleMissingMatchup(row,text)){
      const side=clean(outcome);
      const alreadySelected=/^(Yes|No)$/i.test(side) && text.toLowerCase().endsWith(' '+side.toLowerCase());
      return {game:'Confronto indisponível',bet:text+(side && !alreadySelected?' — '+side:'')};
    }
    return {game:text||'—',bet:clean(outcome)||'—'};
  }
  function kalshiTitleMissingMatchup(row, title = row?.title) {
    if(String(row?._platform||'').toLowerCase()!=='kalshi')return false;
    const text=clean(title);
    if(/\s+(?:vs\.?|versus|@)\s+/i.test(text))return false;
    if(/\s+vs\.?\s+/i.test(clean(row?._kalshiTitleMeta?.matchup)))return true;
    if(/\b(?:reach|score)\s+\d+\s+points?\s+first\??$|\bscores?\s+(?:the\s+)?first\s+(?:TD|touchdown)\??$|\b(?:first|1st)\s+\d+\s+innings?\s+winner\??$|\bneither team\s+(?:wins?|reach)\b/i.test(text))return true;
    return Boolean(row?.sport) && (/^(?:Over|Under)\s+\d+(?:[.,]\d+)?\s+(?:gols|goals?|runs?|pontos|points?)(?:\s+No)?$/i.test(text) || /^.+?\s+wins\??$/i.test(text));
  }
  root.BettyMarketTitles = { clean, cleanKalshiMatchup, kalshiBetQualifier, period, unit, sportUnit, sportFromKalshiRules, footballWinner, inningWinnerTitle, compactKalshiTotal, selectionDetail, kalshiDetail, exactMarket, combatTitle, extendedTitle, kalshiSpecialTitle, kalshiTitleMissingMatchup, positionColumns, drawTitle };
})(globalThis);
