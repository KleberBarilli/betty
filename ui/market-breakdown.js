// Presentation-only grouping. Never derives financial values or mutates bets.
(function(root) {
  const countryAliases = [
    ['Brasil','brazil|brazilian|brasil|brasileir[oa]'],['Estados Unidos','united states|american|u\\.s\\.|usa|u\\.s\\.a\\.'],
    ['Argentina','argentina|argentine|argentinian'],['Canadá','canada|canadian'],['México','mexico|mexican'],
    ['Reino Unido','united kingdom|british|uk|britain'],['França','france|french'],['Alemanha','germany|german'],
    ['Itália','italy|italian'],['Espanha','spain|spanish'],['Portugal','portugal|portuguese'],
    ['Rússia','russia|russian'],['Ucrânia','ukraine|ukrainian'],['China','china|chinese'],['Japão','japan|japanese'],
    ['Índia','india|indian'],['Austrália','australia|australian'],['Israel','israel|israeli'],['Irã','iran|iranian'],
    ['Turquia','turkey|turkiye|turkish'],['Coreia do Sul','south korea|south korean'],['Coreia do Norte','north korea|north korean'],
    ['Chile','chile|chilean'],['Colômbia','colombia|colombian'],['Peru','peru|peruvian'],['Venezuela','venezuela|venezuelan'],
    ['Uruguai','uruguay|uruguayan'],['Equador','ecuador|ecuadorian'],['Bolívia','bolivia|bolivian'],
    ['Países Baixos','netherlands|dutch'],['Polônia','poland|polish'],['Romênia','romania|romanian'],['Hungria','hungary|hungarian'],
    ['África do Sul','south africa|south african'],['Taiwan','taiwan|taiwanese']
  ];
  const normalizeCountryText = value => String(value||'').normalize('NFD').replace(/[\u0300-\u036f]/g,'').toLowerCase();
  const countryPatterns=countryAliases.map(([name,aliases])=>[name,new RegExp(`(?:^|[^a-z])(?:${aliases}|${normalizeCountryText(name)})(?=$|[^a-z])`,'i')]);
  function politicalCountry(title) {
    const text=normalizeCountryText(title),matches=new Set();
    for(const [name,pattern]of countryPatterns)if(pattern.test(text))matches.add(name);
    // Subnational elections are identified by jurisdiction, never candidate names.
    if (/\b(?:governor|governador)\s+(?:of|de|do|da)\s+(?:parana|sao paulo|rio de janeiro|minas gerais|rio grande do sul|santa catarina|bahia|pernambuco|ceara|maranhao|goias|mato grosso|espirito santo|amazonas|para|piaui|sergipe|alagoas|acre|roraima|rondonia|amapa|tocantins|rio grande do norte|paraiba|distrito federal)\b/.test(text))matches.add('Brasil');
    return matches.size===1?[...matches][0]:matches.size>1?'Múltiplos países':'País não identificado';
  }
  function describe(title, category = '') {
    const text = String(title || '').replace(/\s+/g, ' ').trim();
    if (text.includes('Mercado não confirmado:')) return `Outros — ${text.slice(text.indexOf('Mercado não confirmado:'))}`;
    if(category === 'Política')return politicalCountry(text);
    if (category === 'Futebol' && /\b(?:win|reach|qualify|advance)\b/i.test(text)) {
      const competition=text.match(/\b(?:FIFA World Cup|World Cup|Champions League|Europa League|Conference League|Libertadores|Sudamericana|Premier League|La Liga|Serie A|Bundesliga|Ligue 1|Copa do Mundo)\b/i);
      if (competition) {
        const names={'fifa world cup':'Copa do Mundo','world cup':'Copa do Mundo','copa do mundo':'Copa do Mundo'};
        const year=text.match(/\b(?:19|20)\d{2}(?:\s*[-/]\s*(?:\d{2}|\d{4}))?\b/)?.[0] || '';
        return `Longo prazo — ${names[competition[0].toLowerCase()] || competition[0]}${year ? ' '+year : ''}`;
      }
    }
    const detail = text.includes(' - ') ? text.slice(text.indexOf(' - ') + 3) : text;
    const period = detail.match(/\b(?:[12](?:st|nd) Half|[12]H|[1-4]Q|[1-4](?:st|nd|rd|th) Quarter|[1-4][º°] quarto|\d+(?:st|nd|rd|th) Period|[1-3][º°] per[ií]odo|1st \d+ Innings|[12](?:st|nd) Innings|\d+(?:st|nd|rd|th|º) (Inning|inning|set|Map)|1st Set|Set \d+|Extra Innings|Overtime|Extra Time)\b/i);
    const canonicalPeriods = {'1h':'1st Half','2h':'2nd Half','1st half':'1st Half','2nd half':'2nd Half','extra innings':'Extra Innings','overtime':'Overtime','extra time':'Extra Time'};
    for (let n=1;n<=4;n++) {
      const name = `${n}${['','st','nd','rd','th'][n]} Quarter`;
      for (const alias of [name.toLowerCase(),`${n}q`,`${n}º quarto`,`${n}° quarto`]) canonicalPeriods[alias]=name;
    }
    const inningNumber = period?.[0].match(/^(\d+)(?:st|nd|rd|th|º|°)\s+inning$/i);
    if (inningNumber) {
      const n=Number(inningNumber[1]), suffix=n%100>=11&&n%100<=13?'th':({1:'st',2:'nd',3:'rd'}[n%10]||'th');
      canonicalPeriods[period[0].toLowerCase()]=`${n}${suffix} Inning`;
    }
    if (period && /^1st \d+ innings$/i.test(period[0])) canonicalPeriods[period[0].toLowerCase()]=period[0].replace(/innings/i,'Innings');
    const setNumber=period?.[0].match(/^(?:(\d+)(?:st|nd|rd|th|º) set|set (\d+))$/i);
    if(setNumber){const n=Number(setNumber[1]||setNumber[2]);canonicalPeriods[period[0].toLowerCase()]=`${n}${n%100>=11&&n%100<=13?'th':({1:'st',2:'nd',3:'rd'}[n%10]||'th')} Set`;}
    const hockeyPeriod=period?.[0].match(/^([1-3])(?:(?:st|nd|rd) Period|[º°] per[ií]odo)$/i);
    if(hockeyPeriod)canonicalPeriods[period[0].toLowerCase()]=`${hockeyPeriod[1]}${['','st','nd','rd'][Number(hockeyPeriod[1])]} Period`;
    const scope = period ? (canonicalPeriods[period[0].toLowerCase()] || period[0]) : 'Partida inteira';
    const negative = /\bNo\s*$/i.test(detail) ? ' — No' : '';
    if (['Futebol','Basquete','Beisebol'].includes(category) && /\b(?:Chutes no alvo|Assistências|Strikeouts)\s+(?:O\/U|Over|Under)\s+\d/i.test(detail)) return 'Player props';
    const method=detail.match(/Vitória por (nocaute|KO\/TKO|TKO|KO|finalização|decisão)\s+(Yes|No)$/i);
    if(category==='MMA & Boxe' && method)return `Método de vitória — ${method[1][0].toUpperCase()+method[1].slice(1)} — ${method[2]}`;
    if(category==='Automobilismo' && /\bPódio\s+(Yes|No)$/i.test(detail))return `Pódio — ${detail.match(/(Yes|No)$/i)[1]}`;
    if(category==='Golfe' && /^Torneio\s+.+?\s+ML(?:\s+No)?$/i.test(detail))return `ML — Torneio${negative}`;
    if (category === 'Basquete') {
      const prop=detail.match(/^(.+?)\s+(?:rebotes|rebounds?|assist[eê]ncias|assists?|steals?|blocks?|three.pointers?|3.pointers?|PRA)\s+(?:O\/U|Over|Under)\s+\d+(?:[.,]\d+)?(?:\s+(?:Yes|No))?$/i);
      const teams=(text.includes(' - ')?text.slice(0,text.indexOf(' - ')):'').split(/\s+vs\.?\s+/i).map(t=>t.trim().toLowerCase());
      if (prop && !teams.includes(prop[1].trim().toLowerCase())) return 'Player props';
    }
    if (category === 'Basquete' && /:\s*(?:Over|Under|O\/U)\b/i.test(detail) && /\b(?:points?|pontos|rebounds?|rebotes|assists?|assist[eê]ncias|steals?|blocks?|three.pointers?|3.pointers?|PRA)\b/i.test(detail)) {
      const named=detail.split(':')[0].trim().toLowerCase();
      const teams=(text.includes(' - ')?text.slice(0,text.indexOf(' - ')):'').split(/\s+vs\.?\s+/i).map(t=>t.trim().toLowerCase());
      if (!teams.includes(named)) return 'Player props';
    }
    if (/\bML\s*(?:No)?$/i.test(detail) || /^ML\s+/i.test(detail)) return `ML — ${scope}${negative}`;
    if (/\bAdvances\s*(?:No)?$/i.test(detail)) return `Advances${negative}`;
    if (/\bBoth Teams to Score\b/i.test(detail)) return `Ambas marcam — ${scope} — ${/\bNo\s*$/i.test(detail) ? 'No' : /\bYes\s*$/i.test(detail) ? 'Yes' : 'O/U não informado'}`;
    if (/\bEmpate\b/i.test(detail)) return `Empate — ${scope}${negative}`;
    if (category === 'Beisebol' && /^Extra Innings\s+(?:Yes|No)$/i.test(detail)) return `Extra Innings — ${/No$/i.test(detail) ? 'No' : 'Yes'}`;
    const total = detail.match(/\b(Over|Under|O\/U)\s+(\d+(?:[.,]\d+)?)\s+(?:(?:[12]H|[1-4]Q)\s+)?(gols|goals?|pontos|points?|runs|escanteios|corners|games|sets|rebotes|rebounds|yards|rounds?|mapas|maps|wickets|strokes)\b/i);
    if (total) {
      const prefix = detail.slice(0, total.index).replace(period?.[0] || /$^/, '').replace(/[:\s]+/g, ' ').trim();
      const target = prefix ? (/rebotes|rebounds|yards/i.test(total[3]) ? 'Jogador' : /gols|goals|escanteios|corners/i.test(total[3]) ? 'Um time' : 'Equipe/jogador') : 'Total';
      const unit = {goal:'gols',goals:'gols',point:'pontos',points:'pontos',corners:'escanteios',rebounds:'rebotes'}[total[3].toLowerCase()] || total[3].toLowerCase();
      const side = {over:'Over',under:'Under','o/u':'O/U'}[total[1].toLowerCase()];
      if (['MMA & Boxe','Golfe','Automobilismo','Esports','Críquete','Rugby','Vôlei','Outros Esportes'].includes(category) && side !== 'O/U') {
        if (category === 'MMA & Boxe' && unit === 'gols') return `${side} ${total[2]} — Unidade não confirmada`;
        return `${side} ${unit} — ${target} — ${scope}${negative}`;
      }
      if (category === 'Tênis' && ['Over','Under'].includes(side) && ['games','sets'].includes(unit)) {
        const matchup=text.includes(' - ')?text.slice(0,text.indexOf(' - ')):'';
        const participant=matchup.split(/\s+vs\.?\s+/i).some(team=>team.trim().toLowerCase()===prefix.toLowerCase());
        if (!prefix || participant) return `${side} ${unit} — ${prefix ? 'Um jogador/par' : 'Total'} — ${scope}${negative}`;
      }
      if (category === 'Hóquei' && ['Over','Under'].includes(side) && unit === 'gols') {
        const matchup=text.includes(' - ')?text.slice(0,text.indexOf(' - ')):'';
        const team=matchup.split(/\s+vs\.?\s+/i).some(t=>t.trim().toLowerCase()===prefix.toLowerCase());
        if (!prefix || team) return `${side} gols — ${prefix ? 'Um time' : 'Total'} — ${scope}${negative}`;
      }
      if (category === 'Futebol Americano' && unit === 'yards' && prefix && detail.slice(0,total.index).includes(':')) return 'Player props';
      if (['Futebol Americano','Basquete','Beisebol'].includes(category) && ['Over','Under'].includes(side) && unit === (category === 'Beisebol' ? 'runs' : 'pontos')) {
        // Only identify a team when the named participant matches the matchup.
        // Unknown/player prefixes must never be merged into team totals.
        const matchup = text.includes(' - ') ? text.slice(0,text.indexOf(' - ')) : '';
        const teams = matchup.split(/\s+vs\.?\s+/i).map(t => t.trim().toLowerCase());
        const teamTotal = teams.length === 2 && teams.includes(prefix.toLowerCase());
        if (target === 'Total' || teamTotal) return `${side} ${unit} — ${teamTotal ? 'Um time' : 'Total'} — ${scope}` + negative;
      }
      // Soccer goals and corners are compared by side and scope, not numeric line.
      // Periods, team totals and explicit No selections remain separate.
      if (category === 'Futebol' && ['gols','escanteios'].includes(unit) && side !== 'O/U') {
        return `${side} ${unit} — ${target} — ${scope}${negative}`;
      }
      return `${side} ${total[2].replace(',', '.')} ${unit} — ${target} — ${scope}${negative}`;
    }
    if (/\b(?:Handicap|Spread)\b/i.test(detail) || /\s[+-]\d+(?:\.\d+)?\s*$/.test(detail)) {
      const line = detail.match(/[+-]\d+(?:\.\d+)?/);
      const kind = /handicap de sets/i.test(detail) ? 'Handicap de sets' : /handicap de games/i.test(detail) ? 'Handicap de games' : /handicap de mapas/i.test(detail) ? 'Handicap de mapas' : 'Handicap';
      if (['MMA & Boxe','Golfe','Automobilismo','Esports','Críquete','Rugby','Vôlei','Outros Esportes'].includes(category) && line) return `${kind} ${line[0][0] === '-' ? 'Negativo' : 'Positivo'} — ${scope}${negative}`;
      if (['Hóquei','Tênis'].includes(category) && line) return `${kind} ${line[0][0] === '-' ? 'Negativo' : 'Positivo'} — ${scope}${negative}`;
      if (['Futebol','Futebol Americano','Basquete','Beisebol'].includes(category) && kind === 'Handicap' && line) {
        return `Handicap ${line[0][0] === '-' ? 'Negativo' : 'Positivo'} — Total — ${scope === 'Partida inteira' ? 'Partida Inteira' : scope}${negative}`;
      }
      return `${kind}${line ? ' ' + line[0] : ''} — ${scope}${negative}`;
    }
    const race=detail.match(/\bRace to\s+(\d+)\s+pontos\b/i);
    if(race)return `Race${/\bQualquer equipe\b/i.test(detail)?' qualquer equipe':/\bNenhuma equipe\b/i.test(detail)?' nenhuma equipe':''} — ${scope}${negative}`;
    if (/\bfirst TD\b/i.test(detail)) return `First TD — ${scope}${negative}`;
    const incompleteTotal = detail.match(/^(Over|Under)\s+(\d+(?:[.,]\d+)?)$/i);
    if (incompleteTotal) return `${incompleteTotal[1]} ${incompleteTotal[2]} — Unidade/escopo não confirmado`;
    if (/^.+?\s+wins?(?:\s+the)?(?:\s+(?:[12](?:st|nd) Half|[1-4](?:st|nd|rd|th) Quarter))?\s*(?:Yes|No)?$/i.test(detail)
      || (period && /^.+?\s+(?:[12](?:st|nd) Half|[1-4](?:st|nd|rd|th) Quarter|[1-4]º quarto)\s+(?:Yes|No)$/i.test(detail))) return `ML — ${scope}${negative}`;
    // Unsupported formats stay visible rather than being silently guessed as ML.
    return `Outros — ${detail || 'Mercado não identificado'}`;
  }
  const key = (title, category) => JSON.stringify([String(category || 'Outros'), describe(title, category)]);
  function footballOrder(label) {
    const text = String(label || '');
    const corners = /escanteios/i.test(text);
    const family = /^ML\b/.test(text) ? 0 : /^Advances/.test(text) ? 6
      : corners ? 4 : /^Over\b/.test(text) ? 2 : /^Under\b/.test(text) ? 3 : 5;
    const quarter = text.match(/([1-4])(?:st|nd|rd|th) Quarter|([1-4])º quarto/);
    const set = text.match(/(\d+)(?:st|nd|rd|th|º) [Ss]et|Set (\d+)/);
    const inning = text.match(/(\d+)(?:st|nd|rd|th|º) [Ii]nning/);
    const inningsRange = text.match(/1st (\d+) Innings/);
    const hockeyPeriod=text.match(/([1-3])(?:st|nd|rd) Period/);
    const period = /1st Half/.test(text) ? 1 : /2nd Half/.test(text) ? 2
      : /Partida inteira/i.test(text) ? 0 : quarter ? 2 + Number(quarter[1] || quarter[2])
      : hockeyPeriod ? 6 + Number(hockeyPeriod[1]) : set ? 10 + Number(set[1] || set[2]) : inningsRange ? 20 + Math.min(Number(inningsRange[1]),99)/100
      : inning ? 20 + Number(inning[1]) : 100;
    const target = /— Total\b/.test(text) ? 0 : /— Um time\b/.test(text) ? 1 : 2;
    const side = /^Over\b/.test(text) ? 0 : /^Under\b/.test(text) ? 1 : 2;
    const line = Number(text.match(/^(?:Over|Under)\s+(\d+(?:\.\d+)?)/)?.[1] || 0);
    return [family, period, corners ? side : target, corners ? target : line, /— No$/.test(text) ? 1 : 0];
  }
  function compareFootball(a, b) {
    const left = footballOrder(a), right = footballOrder(b);
    for (let i = 0; i < left.length; i++) if (left[i] !== right[i]) return left[i] - right[i];
    return String(a).localeCompare(String(b), 'pt-BR', { numeric: true });
  }
  function aggregateMetrics(rows) {
    const sums = {};
    for (const field of ['total','lucroD','volumeD','lucroStake','volumeStake','wins','losses','oddSoma','oddCount','ganhoBruto','perdaBruta','pnlHoje']) {
      sums[field] = rows.reduce((sum,row) => sum + (Number(row[field]) || 0),0);
    }
    const loss = Math.abs(sums.perdaBruta);
    return {...sums,roi:sums.volumeD > 0 ? sums.lucroD / sums.volumeD * 100 : 0,
      taxaAcerto:sums.total ? sums.wins / sums.total * 100 : 0,
      oddMedia:sums.oddCount ? sums.oddSoma / sums.oddCount : 0,
      avgPnl:sums.total ? sums.lucroD / sums.total : 0,
      avgPnlStake:sums.total ? sums.lucroStake / sums.total : 0,
      profitFactor:loss ? sums.ganhoBruto / loss : sums.ganhoBruto > 0 ? null : 0};
  }
  function expandableGroup(category, label) {
    if (/^Race(?:\s+(?:qualquer equipe|nenhuma equipe|to\s+\d+\s+pontos))?\s+—/i.test(label)) return 'race';
    if(category === 'Futebol' && /^Ambas marcam\b/.test(label))return 'btts';
    if(category === 'Futebol' && /^Longo prazo — /i.test(label))return 'longterm';
    if (category === 'Futebol' && /^Outros — /i.test(label) && /\b(?:win|reach|qualify|advance)\b/i.test(label)
      && /\b(?:World Cup|Champions League|Europa League|Conference League|Libertadores|Sudamericana|Premier League|La Liga|Serie A|Bundesliga|Ligue 1|Copa do Mundo|campeonato|tournament)\b/i.test(label)) return 'longterm';
    if (['MMA & Boxe','Golfe','Automobilismo','Esports','Críquete','Rugby','Vôlei','Outros Esportes'].includes(category)) {
      if (/^ML\b/.test(label))return 'ml';
      if (/^Empate\b/.test(label))return 'draw';
      if (/^Handicap\b/.test(label))return 'handicap';
      if (/Unidade não confirmada/.test(label))return '';
      if (/^Over\b/.test(label))return 'over';
      if (/^Under\b/.test(label))return 'under';
      return '';
    }
    if (['Hóquei','Tênis'].includes(category)) {
      if (/^ML\b/.test(label))return 'ml';
      if (/^Empate\b/.test(label))return 'draw';
      if (/^Handicap\b/.test(label))return 'handicap';
      if (/^Over\b/.test(label) && (category === 'Hóquei' ? /\bgols\b/ : /\b(?:games|sets)\b/).test(label))return 'over';
      if (/^Under\b/.test(label) && (category === 'Hóquei' ? /\bgols\b/ : /\b(?:games|sets)\b/).test(label))return 'under';
      return '';
    }
    if (['Futebol','Beisebol','Futebol Americano'].includes(category) && /^Empate\b/.test(label)) return 'draw';
    if (['Futebol Americano','Basquete','Beisebol'].includes(category)) {
      const unit = category === 'Beisebol' ? /\bruns\b/ : /\bpontos\b/;
      if (/^ML\b/.test(label)) return 'ml';
      if (/^Over\b/.test(label) && unit.test(label)) return 'over';
      if (/^Under\b/.test(label) && unit.test(label)) return 'under';
      if (/^Handicap\b/.test(label)) return 'handicap';
      return '';
    }
    if (category !== 'Futebol') return '';
    if (/^ML\b/.test(label)) return 'ml';
    if (/escanteios/.test(label)) return 'corners';
    if (/^Over\b.*\bgols\b/.test(label)) return 'over';
    if (/^Under\b.*\bgols\b/.test(label)) return 'under';
    if (/^Handicap\b/.test(label)) return 'handicap';
    return '';
  }
  root.BettyMarketBreakdown = Object.freeze({ describe, key, compareFootball, aggregateMetrics, expandableGroup, politicalCountry });
})(globalThis);
