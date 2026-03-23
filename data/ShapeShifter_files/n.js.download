function nc() {
	neopets.templateClock.update();
}

if (typeof(neopets) == 'undefined') neopets = {};

neopets.templateClock = {

	update: function () {
		if(++ns>59){
			ns=0;
			if(++nm>59){
				nm=0;
				if(++nh>12){
					nh=1;
				}
				else if(nh==12){
					na=na=='am'?'pm':'am';
				}
			}
		}
		var nbj=document.getElementById('nst');
		if(nbj==null){
			return;
		}
		nbj.innerHTML=this.nstTime(nh,nm,ns,na);
	},

	nstTime: function (nh,nm,ns,na) {
		if(nl=='fr'){
			nz='HSN';
					nk='Heure Standard Neopienne';
		}
		else if(nl=='de'){
			nz='NSZ';
					nk='Neopianische Standard-Zeit';
		}
		else if(nl=='es'){
			nz='NST';
					nk='Hora est\u00e1ndar neopiana';
		}
		else if(nl=='pt'){
			nz='NST';
					nk='Hor\u00e1rio Oficial de Neopia';
		}
		else if(nl=='it'){
			nz='NST';
					nk='Ora Neopiana';
		}
		else if(nl=='nl'){
			nz='NST';
					nk='Neopische Standaard Tijd';
		}
		else if(nl=='ch'){
			nz='NST';
					nk='\u5c3c\u5965\u4e16\u754c\u6807\u51c6\u65f6\u95f4';
		}
		else if(nl=='zh'){
			nz='NST';
					nk='\u5c3c\u5967\u4e16\u754c\u6a19\u6e96\u6642\u9593';
		}
		else if(nl=='ja'){
			nz='NST';
					nk='\u30cd\u30aa\u30d4\u30a2\u6a19\u6e96\u6642\u523b';
		}
		else if(nl=='ko'){
			nz='NST';
					nk='\ub124\uc624\ud53c\uc548 \ud45c\uc900 \uc2dc\uac04';
		}
		else{
			nz='NST';
		}

			if(nl!='en'){
					nz='<span alt="'+nk+'" title="'+nk+'" style="cursor:hand;cursor:pointer">'+nz+'</span>';
			}

		if(nl=='pt'||nl=='fr'||nl=='es'||nl=='it'||nl=='nl'||nl=='de'){
			if(na=='pm'&&nh<12){
				nh+=12;
			}
			else if(na=='am'&&nh==12){
				nh=0;
			}
			ret=nh+':'+this.t(nm)+':'+this.t(ns)+' '+nz;
		}
		else if(nl=='ch'||nl=='zh'||nl=='ja'){
			ret=nz+' '+nh+':'+this.t(nm)+':'+this.t(ns)+' '+na;
		}
		else if(nl=='ko'){
			ret=nz+' '+na+' '+nh+':'+this.t(nm)+':'+this.t(ns);
		}
		else{
			ret=nh+':'+this.t(nm)+':'+this.t(ns)+' '+na+' '+nz;
		}
		return ret;
	},

	t: function (c){
		if (c<=9) {
			r1='0'+c;
		} else {
			r1=c;
		}
		return r1;
	}

}
